use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use rand::Rng;
use reqwest::Client;

use super::metrics::LoadMetrics;

pub async fn submit_solver_bid(
    client: &Client,
    base_url: &str,
    intent_id: &str,
    solver_index: u64,
    metrics: &Arc<LoadMetrics>,
) {
    let (amount_out, fee) = {
        let mut rng = rand::rng();
        (rng.random_range(100u64..15_000), rng.random_range(1u64..50))
    };
    let solver_id = format!("solver-{solver_index}");

    let start = Instant::now();
    let resp = client
        .post(format!("{base_url}/bids"))
        .json(&serde_json::json!({
            "intent_id": intent_id,
            "solver_id": solver_id,
            "amount_out": amount_out,
            "fee": fee,
        }))
        .send()
        .await;

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    metrics.record_bid_latency(latency_ms).await;

    match resp {
        Ok(r) if r.status().is_success() => {
            metrics.bids_sent.fetch_add(1, Ordering::Relaxed);
        }
        Ok(r) => {
            metrics.bids_failed.fetch_add(1, Ordering::Relaxed);
            let body = r.text().await.unwrap_or_default();
            eprintln!("  Bid failed: {body}");
        }
        Err(e) => {
            metrics.bids_failed.fetch_add(1, Ordering::Relaxed);
            eprintln!("  Bid error: {e}");
        }
    }
}
