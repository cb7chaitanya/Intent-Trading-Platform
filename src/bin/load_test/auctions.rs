use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::metrics::LoadMetrics;

/// Connect to WS feed, count messages and measure inter-message latency.
pub async fn ws_subscriber(
    ws_url: &str,
    metrics: Arc<LoadMetrics>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let conn = connect_async(ws_url).await;
    let (ws_stream, _) = match conn {
        Ok(c) => {
            metrics.ws_connections.fetch_add(1, Ordering::Relaxed);
            c
        }
        Err(e) => {
            metrics.ws_errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("  WS connect failed: {e}");
            return;
        }
    };

    let (_write, mut read) = ws_stream.split();
    let mut last_msg = Instant::now();

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(_))) => {
                        let now = Instant::now();
                        let gap_ms = (now - last_msg).as_secs_f64() * 1000.0;
                        last_msg = now;
                        metrics.ws_messages.fetch_add(1, Ordering::Relaxed);
                        // Record inter-message gap as WS latency proxy
                        if gap_ms < 30_000.0 {
                            metrics.record_ws_latency(gap_ms).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        metrics.ws_errors.fetch_add(1, Ordering::Relaxed);
                        eprintln!("  WS error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            _ = shutdown.changed() => {
                break;
            }
        }
    }
}

/// Poll intent status until terminal state, measuring end-to-end settlement latency.
pub async fn track_settlement(
    client: &reqwest::Client,
    base_url: &str,
    intent_id: &str,
    token: &str,
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
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await;

        if let Ok(r) = resp {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let status = body["status"].as_str().unwrap_or("");
                match status {
                    "Completed" | "PartiallyFilled" => {
                        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                        metrics.record_settlement_latency(latency_ms).await;
                        metrics.settlements_ok.fetch_add(1, Ordering::Relaxed);
                        metrics.trades_executed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    "Failed" | "Cancelled" | "Expired" => {
                        metrics.settlements_failed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    _ => {} // still in progress
                }
            }
        }
    }
}

/// Query orderbook for a market, measure latency.
pub async fn query_orderbook(
    client: &reqwest::Client,
    base_url: &str,
    market_id: &str,
    metrics: &Arc<LoadMetrics>,
) {
    let start = Instant::now();
    let resp = client
        .get(format!("{base_url}/orderbook/{market_id}"))
        .send()
        .await;

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    metrics.record_orderbook_latency(latency_ms).await;

    match resp {
        Ok(r) if r.status().is_success() => {
            metrics.orderbook_queries.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            metrics.orderbook_failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}
