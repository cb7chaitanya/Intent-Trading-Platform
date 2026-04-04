use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

use crate::db::redis::{
    Event, BID_SUBMITTED, EXECUTION_COMPLETED, EXECUTION_STARTED, INTENT_CREATED, INTENT_MATCHED,
};

const WS_CHANNELS: &[&str] = &[
    INTENT_CREATED,
    BID_SUBMITTED,
    INTENT_MATCHED,
    EXECUTION_STARTED,
    EXECUTION_COMPLETED,
];

#[derive(Clone)]
pub struct WsServer {
    tx: broadcast::Sender<String>,
}

impl WsServer {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn broadcast(&self, message: &str) {
        // Ignore error when there are no active receivers
        let _ = self.tx.send(message.to_string());
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/ws", get(ws_handler))
            .with_state(Arc::new(self))
    }

    pub async fn start_redis_listener(&self, redis_client: &redis::Client) {
        let mut pubsub = match redis_client.get_async_pubsub().await {
            Ok(ps) => ps,
            Err(e) => {
                tracing::error!(error = %e, "Failed to connect to Redis for WS listener");
                return;
            }
        };

        for channel in WS_CHANNELS {
            if let Err(e) = pubsub.subscribe(*channel).await {
                tracing::error!(channel = %channel, error = %e, "Failed to subscribe WS channel");
                return;
            }
        }

        let tx = self.tx.clone();
        let mut stream = pubsub.on_message();

        while let Some(msg) = stream.next().await {
            let channel = msg.get_channel_name().to_string();
            let payload: String = match msg.get_payload() {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to read WS message payload");
                    continue;
                }
            };

            match serde_json::from_str::<Event>(&payload) {
                Ok(event) => {
                    let ws_message = serde_json::json!({
                        "event": channel,
                        "data": event,
                    });
                    let _ = tx.send(ws_message.to_string());
                }
                Err(e) => {
                    tracing::warn!(channel = %channel, error = %e, "Failed to deserialize WS event");
                }
            }
        }
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(server): State<Arc<WsServer>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, server))
}

async fn handle_socket(socket: WebSocket, server: Arc<WsServer>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = server.tx.subscribe();

    // Forward broadcast messages to this client
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Wait for client disconnect
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(_)) = receiver.next().await {}
    });

    // When either task ends, abort the other
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}
