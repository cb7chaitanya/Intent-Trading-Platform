use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use rand::Rng;
use reqwest::Client;

use super::metrics::LoadMetrics;
use super::users::TestUser;

const TOKENS: &[(&str, &str)] = &[
    ("ETH", "USDC"),
    ("BTC", "USDC"),
    ("SOL", "USDC"),
];

pub async fn submit_random_intent(
    client: &Client,
    base_url: &str,
    user: &TestUser,
    metrics: &Arc<LoadMetrics>,
) -> Option<String> {
    let (token_in, token_out, amount_in, min_amount_out) = {
        let mut rng = rand::rng();
        let pair = TOKENS[rng.random_range(0..TOKENS.len())];
        let amt: u64 = rng.random_range(100..10_000);
        let min_out: u64 = (amt as f64 * rng.random_range(0.9..1.1)) as u64;
        (pair.0, pair.1, amt, min_out)
    };
    let deadline = chrono::Utc::now().timestamp() + 3600;

    let start = Instant::now();
    let resp = client
        .post(format!("{base_url}/intents"))
        .json(&serde_json::json!({
            "user_id": user.user_id,
            "account_id": user.account_id,
            "token_in": token_in,
            "token_out": token_out,
            "amount_in": amount_in,
            "min_amount_out": min_amount_out,
            "deadline": deadline,
        }))
        .send()
        .await;

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    metrics.record_intent_latency(latency_ms).await;

    match resp {
        Ok(r) if r.status().is_success() => {
            metrics.intents_sent.fetch_add(1, Ordering::Relaxed);
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            body["id"].as_str().map(|s| s.to_string())
        }
        Ok(r) => {
            metrics.intents_failed.fetch_add(1, Ordering::Relaxed);
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            eprintln!("  Intent failed: {status} {body}");
            None
        }
        Err(e) => {
            metrics.intents_failed.fetch_add(1, Ordering::Relaxed);
            eprintln!("  Intent error: {e}");
            None
        }
    }
}
