use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use redis::{AsyncCommands, Client, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Stream keys
pub const STREAM_INTENT_CREATED: &str = "stream:intent.created";
pub const STREAM_BID_SUBMITTED: &str = "stream:bid.submitted";
pub const STREAM_AUCTION_ENDED: &str = "stream:auction.ended";
pub const STREAM_TRADE_EXECUTED: &str = "stream:trade.executed";
pub const STREAM_TRADE_SETTLED: &str = "stream:trade.settled";
pub const STREAM_BALANCE_UPDATED: &str = "stream:balance.updated";
pub const STREAM_MARKET_TRADE: &str = "stream:market.trade";
pub const STREAM_EXECUTION_COMPLETED: &str = "stream:execution.completed";
pub const STREAM_INTENT_SETTLED: &str = "stream:intent.settled";

pub const ALL_STREAMS: &[&str] = &[
    STREAM_INTENT_CREATED,
    STREAM_BID_SUBMITTED,
    STREAM_AUCTION_ENDED,
    STREAM_TRADE_EXECUTED,
    STREAM_TRADE_SETTLED,
    STREAM_BALANCE_UPDATED,
    STREAM_MARKET_TRADE,
    STREAM_EXECUTION_COMPLETED,
    STREAM_INTENT_SETTLED,
];

/// A typed stream event with JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event_type: String,
    pub payload: String,
    pub event_id: String,
    pub timestamp: i64,
}

/// Maximum stream length before trimming old entries.
const MAX_STREAM_LEN: usize = 10_000;

/// Redis Streams-backed event bus with consumer groups.
#[derive(Clone)]
pub struct StreamBus {
    client: Client,
    conn: MultiplexedConnection,
}

impl StreamBus {
    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = Client::open(redis_url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { client, conn })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Publish a JSON-serializable payload to a stream.
    pub async fn publish<T: Serialize>(
        &self,
        stream: &str,
        payload: &T,
    ) -> Result<String, redis::RedisError> {
        let json = serde_json::to_string(payload).map_err(|e| {
            redis::RedisError::from((
                redis::ErrorKind::TypeError,
                "JSON serialization failed",
                e.to_string(),
            ))
        })?;

        let event_id = Uuid::new_v4().to_string();
        let ts = chrono::Utc::now().timestamp();

        let mut conn = self.conn.clone();

        // XADD stream MAXLEN ~ 10000 * event_type payload event_id timestamp
        let id: String = redis::cmd("XADD")
            .arg(stream)
            .arg("MAXLEN")
            .arg("~")
            .arg(MAX_STREAM_LEN)
            .arg("*")
            .arg("event_type")
            .arg(stream)
            .arg("payload")
            .arg(&json)
            .arg("event_id")
            .arg(&event_id)
            .arg("timestamp")
            .arg(ts)
            .query_async(&mut conn)
            .await?;

        tracing::debug!(stream, event_id, redis_id = %id, "published event");
        Ok(id)
    }

    /// Create a consumer group for a stream. Idempotent — ignores if it exists.
    pub async fn ensure_group(
        &self,
        stream: &str,
        group: &str,
    ) -> Result<(), redis::RedisError> {
        let mut conn = self.conn.clone();

        // Create the stream with a dummy entry if it doesn't exist, then trim
        let exists: bool = redis::cmd("EXISTS")
            .arg(stream)
            .query_async(&mut conn)
            .await?;

        if !exists {
            let _: String = redis::cmd("XADD")
                .arg(stream)
                .arg("*")
                .arg("_init")
                .arg("1")
                .query_async(&mut conn)
                .await?;
        }

        let result: Result<(), redis::RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        match result {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("BUSYGROUP") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Subscribe to a stream using a consumer group. Calls `handler` for each
    /// event. Automatically ACKs processed messages.
    ///
    /// `group` — consumer group name (e.g. "auction-engine")
    /// `consumer` — unique consumer name within the group
    pub async fn subscribe<F, Fut>(
        &self,
        streams: &[&str],
        group: &str,
        consumer: &str,
        handler: F,
    ) -> Result<(), redis::RedisError>
    where
        F: Fn(StreamEvent) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut conn = self.conn.clone();

        // Ensure groups exist
        for stream in streams {
            self.ensure_group(stream, group).await?;
        }

        loop {
            // Build XREADGROUP command for all streams
            let mut cmd = redis::cmd("XREADGROUP");
            cmd.arg("GROUP")
                .arg(group)
                .arg(consumer)
                .arg("COUNT")
                .arg(10)
                .arg("BLOCK")
                .arg(5000_u64)
                .arg("STREAMS");

            for stream in streams {
                cmd.arg(*stream);
            }
            for _ in streams {
                cmd.arg(">"); // Only new messages
            }

            let result: Value = cmd.query_async(&mut conn).await?;

            let entries = parse_xreadgroup_result(&result);
            for (stream_name, events) in entries {
                for (redis_id, event) in events {
                    handler(event).await;

                    // ACK
                    let _: i64 = redis::cmd("XACK")
                        .arg(&stream_name)
                        .arg(group)
                        .arg(&redis_id)
                        .query_async(&mut conn)
                        .await
                        .unwrap_or(0);
                }
            }
        }
    }

    /// Read recent entries from a stream (no consumer group, just raw read).
    pub async fn read_recent(
        &self,
        stream: &str,
        count: usize,
    ) -> Result<Vec<StreamEvent>, redis::RedisError> {
        let mut conn = self.conn.clone();

        let result: Value = redis::cmd("XREVRANGE")
            .arg(stream)
            .arg("+")
            .arg("-")
            .arg("COUNT")
            .arg(count)
            .query_async(&mut conn)
            .await?;

        Ok(parse_xrange_result(&result))
    }
}

/// Parse XREADGROUP response into (stream_name, [(redis_id, StreamEvent)])
fn parse_xreadgroup_result(value: &Value) -> Vec<(String, Vec<(String, StreamEvent)>)> {
    let mut result = Vec::new();

    let streams = match value {
        Value::Array(arr) => arr,
        _ => return result,
    };

    for stream_entry in streams {
        let parts = match stream_entry {
            Value::Array(arr) if arr.len() == 2 => arr,
            _ => continue,
        };

        let stream_name = match &parts[0] {
            Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
            _ => continue,
        };

        let messages = match &parts[1] {
            Value::Array(arr) => arr,
            _ => continue,
        };

        let mut events = Vec::new();
        for msg in messages {
            let msg_parts = match msg {
                Value::Array(arr) if arr.len() == 2 => arr,
                _ => continue,
            };

            let redis_id = match &msg_parts[0] {
                Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                _ => continue,
            };

            let fields = match &msg_parts[1] {
                Value::Array(arr) => parse_field_pairs(arr),
                _ => continue,
            };

            if let Some(event) = fields_to_event(&fields) {
                events.push((redis_id, event));
            }
        }

        result.push((stream_name, events));
    }

    result
}

/// Parse XRANGE/XREVRANGE response into Vec<StreamEvent>
fn parse_xrange_result(value: &Value) -> Vec<StreamEvent> {
    let messages = match value {
        Value::Array(arr) => arr,
        _ => return Vec::new(),
    };

    let mut events = Vec::new();
    for msg in messages {
        let msg_parts = match msg {
            Value::Array(arr) if arr.len() == 2 => arr,
            _ => continue,
        };

        let fields = match &msg_parts[1] {
            Value::Array(arr) => parse_field_pairs(arr),
            _ => continue,
        };

        if let Some(event) = fields_to_event(&fields) {
            events.push(event);
        }
    }

    events
}

fn parse_field_pairs(arr: &[Value]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for chunk in arr.chunks(2) {
        if chunk.len() == 2 {
            let key = match &chunk[0] {
                Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                _ => continue,
            };
            let val = match &chunk[1] {
                Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                _ => continue,
            };
            map.insert(key, val);
        }
    }
    map
}

fn fields_to_event(fields: &HashMap<String, String>) -> Option<StreamEvent> {
    Some(StreamEvent {
        event_type: fields.get("event_type")?.clone(),
        payload: fields.get("payload")?.clone(),
        event_id: fields.get("event_id").cloned().unwrap_or_default(),
        timestamp: fields
            .get("timestamp")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0),
    })
}
