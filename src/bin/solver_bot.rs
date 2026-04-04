use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use intent_trading::solver::positions::PositionTracker;

// ---------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------

struct Config {
    server_url: String,
    ws_url: String,
    database_url: String,
    api_key: Option<String>,
    solver_id: String,
    bid_strategy: BidStrategy,
    max_position: i64,
    poll_interval_ms: u64,
}

#[derive(Debug, Clone, Copy)]
enum BidStrategy { Aggressive, Conservative, Balanced }

impl Config {
    fn from_env() -> Self {
        let _ = dotenvy::dotenv();
        let strategy = match std::env::var("BID_STRATEGY").unwrap_or_else(|_| "balanced".into()).to_lowercase().as_str() {
            "aggressive" => BidStrategy::Aggressive,
            "conservative" => BidStrategy::Conservative,
            _ => BidStrategy::Balanced,
        };
        Self {
            server_url: std::env::var("SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into()),
            ws_url: std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:3000/ws".into()),
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/intent_trading".into()),
            api_key: std::env::var("API_KEY").ok(),
            solver_id: std::env::var("SOLVER_ID").unwrap_or_else(|_| "solver-bot-alpha".into()),
            bid_strategy: strategy,
            max_position: std::env::var("MAX_POSITION").ok().and_then(|v| v.parse().ok()).unwrap_or(1_000_000),
            poll_interval_ms: std::env::var("POLL_INTERVAL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(100),
        }
    }
    fn tag(&self) -> &str { &self.solver_id }
}

// ---------------------------------------------------------------
// Models
// ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WsEvent { event: String, data: serde_json::Value }

#[derive(Debug, Deserialize)]
struct Intent {
    id: Uuid, #[allow(dead_code)] user_id: String,
    token_in: String, token_out: String,
    amount_in: u64, min_amount_out: u64,
    #[allow(dead_code)] deadline: i64, #[allow(dead_code)] status: String, #[allow(dead_code)] created_at: i64,
}

#[derive(Debug, Serialize)]
struct BidRequest { intent_id: Uuid, solver_id: String, amount_out: u64, fee: u64 }

#[derive(Debug, Deserialize)]
struct BidResponse { id: Uuid, #[allow(dead_code)] intent_id: Uuid, #[allow(dead_code)] solver_id: String, amount_out: u64, fee: u64, #[allow(dead_code)] timestamp: i64 }

// ---------------------------------------------------------------
// Main
// ---------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
    println!("[{}] Starting solver bot", cfg.tag());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&cfg.database_url)
        .await
        .expect("Failed to connect to DB for position tracking");

    let tracker = PositionTracker::new(pool);

    // Load existing positions
    let positions = tracker.load_positions(&cfg.solver_id).await.unwrap_or_default();
    let exposure = tracker.get_total_exposure(&cfg.solver_id).await;
    println!("[{}] Positions: {} assets, exposure: {}/{}", cfg.tag(), positions.len(), exposure, cfg.max_position);
    for p in &positions {
        println!("[{}]   {}: qty={} avg={} rpnl={}", cfg.tag(), p.asset, p.position, p.avg_entry_price, p.realized_pnl);
    }

    loop {
        match run(&cfg, &tracker).await {
            Ok(()) => println!("[{}] WebSocket closed, reconnecting...", cfg.tag()),
            Err(e) => eprintln!("[{}] Error: {e}, reconnecting in 3s...", cfg.tag()),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

async fn run(cfg: &Config, tracker: &PositionTracker) -> Result<(), Box<dyn std::error::Error>> {
    let (ws_stream, _) = connect_async(&cfg.ws_url).await?;
    println!("[{}] Connected", cfg.tag());
    let (mut _write, mut read) = ws_stream.split();
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build()?;

    while let Some(msg) = read.next().await {
        let msg = match msg { Ok(Message::Text(t)) => t.to_string(), Ok(Message::Close(_)) => break, Ok(_) => continue, Err(e) => { eprintln!("[{}] WS: {e}", cfg.tag()); break; } };
        let ws_event: WsEvent = match serde_json::from_str(&msg) { Ok(e) => e, Err(_) => continue };

        match ws_event.event.as_str() {
            "intent_created" => handle_new_intent(cfg, &http, tracker, &ws_event.data).await,
            "intent_matched" => handle_matched(cfg, tracker, &ws_event.data).await,
            "execution_completed" => handle_execution_completed(cfg, &ws_event.data),
            _ => {}
        }

        if cfg.poll_interval_ms > 0 { tokio::time::sleep(tokio::time::Duration::from_millis(cfg.poll_interval_ms)).await; }
    }
    Ok(())
}

// ---------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------

async fn handle_new_intent(cfg: &Config, http: &reqwest::Client, tracker: &PositionTracker, data: &serde_json::Value) {
    let intent: Intent = match data.get("data").and_then(|v| serde_json::from_value(v.clone()).ok()) {
        Some(i) => i, None => return,
    };

    if !tracker.check_limit(&cfg.solver_id, intent.amount_in as i64, cfg.max_position).await {
        let exp = tracker.get_total_exposure(&cfg.solver_id).await;
        println!("[{}] Skip {} — exposure {}/{}", cfg.tag(), intent.id, exp, cfg.max_position);
        return;
    }

    let (premium_range, fee_pct) = match cfg.bid_strategy {
        BidStrategy::Aggressive => (1.05..1.10, 0.002),
        BidStrategy::Conservative => (1.00..1.03, 0.008),
        BidStrategy::Balanced => (1.00..1.10, 0.005),
    };
    let (amount_out, fee) = { let mut r = rand::rng(); ((intent.min_amount_out as f64 * r.random_range(premium_range)) as u64, (intent.amount_in as f64 * fee_pct) as u64) };

    let bid = BidRequest { intent_id: intent.id, solver_id: cfg.solver_id.clone(), amount_out, fee };
    let mut req = http.post(format!("{}/bids", cfg.server_url)).json(&bid);
    if let Some(k) = &cfg.api_key { req = req.header("x-api-key", k); }

    match req.send().await {
        Ok(r) if r.status().is_success() => { if let Ok(b) = r.json::<BidResponse>().await { println!("[{}] Bid: id={} out={} fee={}", cfg.tag(), b.id, b.amount_out, b.fee); } }
        Ok(r) => { let s = r.status(); let b = r.text().await.unwrap_or_default(); eprintln!("[{}] Rejected: {s} {b}", cfg.tag()); }
        Err(e) => eprintln!("[{}] Failed: {e}", cfg.tag()),
    }
}

async fn handle_matched(cfg: &Config, tracker: &PositionTracker, data: &serde_json::Value) {
    let d = data.get("data").unwrap_or(data);
    if d.get("bid").and_then(|b| b.get("solver_id")).and_then(|s| s.as_str()) != Some(&cfg.solver_id) { return; }

    let token_in = d.get("intent").and_then(|i| i.get("token_in")).and_then(|s| s.as_str()).unwrap_or("?");
    let amount_in = d.get("intent").and_then(|i| i.get("amount_in")).and_then(|v| v.as_i64()).unwrap_or(0);
    let amount_out = d.get("bid").and_then(|b| b.get("amount_out")).and_then(|v| v.as_i64()).unwrap_or(0);
    let fee = d.get("bid").and_then(|b| b.get("fee")).and_then(|v| v.as_i64()).unwrap_or(0);
    let net = amount_out - fee;
    let price = if net > 0 { amount_in * 100 / net } else { 0 };

    match tracker.record_fill(&cfg.solver_id, token_in, amount_in, price).await {
        Ok(pos) => println!("[{}] WON: pos={} avg={} rpnl={}", cfg.tag(), pos.position, pos.avg_entry_price, pos.realized_pnl),
        Err(e) => eprintln!("[{}] Position update failed: {e}", cfg.tag()),
    }
}

fn handle_execution_completed(cfg: &Config, data: &serde_json::Value) {
    let d = data.get("data").unwrap_or(data);
    if d.get("solver_id").and_then(|s| s.as_str()) != Some(&cfg.solver_id) { return; }
    let id = d.get("intent_id").and_then(|s| s.as_str()).unwrap_or("?");
    println!("[{}] Executed: {id}", cfg.tag());
}
