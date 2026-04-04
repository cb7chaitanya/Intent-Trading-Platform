use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::metrics::LoadMetrics;

/// Connect to WS, subscribe to events, count messages until cancelled.
pub async fn ws_subscriber(
    ws_url: &str,
    metrics: Arc<LoadMetrics>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let conn = connect_async(ws_url).await;
    let (ws_stream, _) = match conn {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  WS connect failed: {e}");
            return;
        }
    };

    let (mut _write, mut read) = ws_stream.split();

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(_))) => {
                        metrics.ws_messages.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            _ = shutdown.changed() => {
                break;
            }
        }
    }
}

/// Monitor intents via the API and track when they reach Completed/Failed.
pub async fn track_settlements(
    client: &reqwest::Client,
    base_url: &str,
    intent_id: &str,
    metrics: &Arc<LoadMetrics>,
    timeout_secs: u64,
) {
    let start = Instant::now();
    let deadline = std::time::Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() > deadline {
            metrics.settlements_failed.fetch_add(1, Ordering::Relaxed);
            return;
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let resp = client
            .get(format!("{base_url}/intents/{intent_id}"))
            .send()
            .await;

        if let Ok(r) = resp {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let status = body["status"].as_str().unwrap_or("");
                match status {
                    "Completed" => {
                        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                        metrics.record_settlement_latency(latency_ms).await;
                        metrics.settlements_ok.fetch_add(1, Ordering::Relaxed);
                        metrics.trades_executed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    "Failed" | "Cancelled" => {
                        metrics.settlements_failed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    _ => {} // Still in progress
                }
            }
        }
    }
}
