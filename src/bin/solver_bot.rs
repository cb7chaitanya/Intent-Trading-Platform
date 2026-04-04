use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

// ---------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------

struct Config {
    server_url: String,
    ws_url: String,
    api_key: Option<String>,
    solver_id: String,
    bid_strategy: BidStrategy,
    max_position: u64,
    poll_interval_ms: u64,
}

#[derive(Debug, Clone, Copy)]
enum BidStrategy {
    Aggressive,  // 105-110% of min_amount_out, low fee
    Conservative, // 100-103% of min_amount_out, higher fee
    Balanced,     // 100-110% of min_amount_out, medium fee
}

impl Config {
    fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let strategy = match std::env::var("BID_STRATEGY")
            .unwrap_or_else(|_| "balanced".into())
            .to_lowercase()
            .as_str()
        {
            "aggressive" => BidStrategy::Aggressive,
            "conservative" => BidStrategy::Conservative,
            _ => BidStrategy::Balanced,
        };

        Self {
            server_url: std::env::var("SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3000".into()),
            ws_url: std::env::var("WS_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:3000/ws".into()),
            api_key: std::env::var("API_KEY").ok(),
            solver_id: std::env::var("SOLVER_ID")
                .unwrap_or_else(|_| "solver-bot-alpha".into()),
            bid_strategy: strategy,
            max_position: std::env::var("MAX_POSITION")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_000_000),
            poll_interval_ms: std::env::var("POLL_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
        }
    }

    fn tag(&self) -> &str {
        &self.solver_id
    }
}

// ---------------------------------------------------------------
// Models
// ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WsEvent {
    event: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Intent {
    id: Uuid,
    #[allow(dead_code)]
    user_id: String,
    token_in: String,
    token_out: String,
    amount_in: u64,
    min_amount_out: u64,
    #[allow(dead_code)]
    deadline: i64,
    #[allow(dead_code)]
    status: String,
    #[allow(dead_code)]
    created_at: i64,
}

#[derive(Debug, Serialize)]
struct BidRequest {
    intent_id: Uuid,
    solver_id: String,
    amount_out: u64,
    fee: u64,
}

#[derive(Debug, Deserialize)]
struct BidResponse {
    id: Uuid,
    #[allow(dead_code)]
    intent_id: Uuid,
    #[allow(dead_code)]
    solver_id: String,
    amount_out: u64,
    fee: u64,
    #[allow(dead_code)]
    timestamp: i64,
}

// ---------------------------------------------------------------
// Main
// ---------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();

    println!("[{}] Starting solver bot", cfg.tag());
    println!("[{}]   server:   {}", cfg.tag(), cfg.server_url);
    println!("[{}]   ws:       {}", cfg.tag(), cfg.ws_url);
    println!("[{}]   strategy: {:?}", cfg.tag(), cfg.bid_strategy);
    println!("[{}]   max_pos:  {}", cfg.tag(), cfg.max_position);
    println!("[{}]   api_key:  {}", cfg.tag(), if cfg.api_key.is_some() { "set" } else { "none" });

    loop {
        match run(&cfg).await {
            Ok(()) => println!("[{}] WebSocket closed, reconnecting...", cfg.tag()),
            Err(e) => eprintln!("[{}] Error: {e}, reconnecting in 3s...", cfg.tag()),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

async fn run(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    println!("[{}] Connecting to {}...", cfg.tag(), cfg.ws_url);
    let (ws_stream, _) = connect_async(&cfg.ws_url).await?;
    println!("[{}] Connected to WebSocket", cfg.tag());

    let (mut _write, mut read) = ws_stream.split();

    let mut http_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10));

    let http = http_builder.build()?;
    let mut position: u64 = 0;

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                eprintln!("[{}] WS read error: {e}", cfg.tag());
                break;
            }
        };

        let ws_event: WsEvent = match serde_json::from_str(&msg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[{}] Failed to parse event: {e}", cfg.tag());
                continue;
            }
        };

        match ws_event.event.as_str() {
            "intent_created" => {
                handle_new_intent(cfg, &http, &ws_event.data, &mut position).await;
            }
            "intent_matched" => {
                handle_matched(cfg, &ws_event.data);
            }
            "execution_completed" => {
                handle_execution_completed(cfg, &ws_event.data);
            }
            _ => {}
        }

        // Throttle
        if cfg.poll_interval_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(cfg.poll_interval_ms)).await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------

async fn handle_new_intent(
    cfg: &Config,
    http: &reqwest::Client,
    data: &serde_json::Value,
    position: &mut u64,
) {
    let intent_value = match data.get("data") {
        Some(v) => v,
        None => return,
    };

    let intent: Intent = match serde_json::from_value(intent_value.clone()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[{}] Failed to parse intent: {e}", cfg.tag());
            return;
        }
    };

    // Position limit check
    if *position + intent.amount_in > cfg.max_position {
        println!(
            "[{}] Skipping intent {} — would exceed max position ({} + {} > {})",
            cfg.tag(), intent.id, position, intent.amount_in, cfg.max_position
        );
        return;
    }

    println!(
        "[{}] New intent: {} | {} -> {} | amount: {}",
        cfg.tag(), intent.id, intent.token_in, intent.token_out, intent.amount_in
    );

    // Generate quote based on strategy
    let (premium_range, fee_pct) = match cfg.bid_strategy {
        BidStrategy::Aggressive => (1.05..1.10, 0.002),
        BidStrategy::Conservative => (1.00..1.03, 0.008),
        BidStrategy::Balanced => (1.00..1.10, 0.005),
    };

    let (amount_out, fee) = {
        let mut rng = rand::rng();
        let premium: f64 = rng.random_range(premium_range);
        let amount_out = (intent.min_amount_out as f64 * premium) as u64;
        let fee = (intent.amount_in as f64 * fee_pct) as u64;
        (amount_out, fee)
    };

    let profit = amount_out.saturating_sub(intent.min_amount_out).saturating_sub(fee);
    println!(
        "[{}] Quoting: amount_out={amount_out}, fee={fee}, est_profit={profit}",
        cfg.tag()
    );

    let bid = BidRequest {
        intent_id: intent.id,
        solver_id: cfg.solver_id.clone(),
        amount_out,
        fee,
    };

    let mut req = http.post(format!("{}/bids", cfg.server_url)).json(&bid);
    if let Some(key) = &cfg.api_key {
        req = req.header("x-api-key", key);
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<BidResponse>().await {
                Ok(bid_resp) => {
                    *position += intent.amount_in;
                    println!(
                        "[{}] Bid submitted: id={}, amount_out={}, fee={}",
                        cfg.tag(), bid_resp.id, bid_resp.amount_out, bid_resp.fee
                    );
                }
                Err(e) => eprintln!("[{}] Failed to parse bid response: {e}", cfg.tag()),
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            eprintln!("[{}] Bid rejected: {status} - {body}", cfg.tag());
        }
        Err(e) => eprintln!("[{}] Failed to submit bid: {e}", cfg.tag()),
    }
}

fn handle_matched(cfg: &Config, data: &serde_json::Value) {
    let matched_data = data.get("data").unwrap_or(data);

    let bid_solver = matched_data
        .get("bid")
        .and_then(|b| b.get("solver_id"))
        .and_then(|s| s.as_str());

    let intent_id = matched_data
        .get("intent")
        .and_then(|i| i.get("id"))
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");

    if bid_solver == Some(&cfg.solver_id) {
        let amount_out = matched_data.get("bid").and_then(|b| b.get("amount_out")).and_then(|v| v.as_u64()).unwrap_or(0);
        let fee = matched_data.get("bid").and_then(|b| b.get("fee")).and_then(|v| v.as_u64()).unwrap_or(0);
        println!("[{}] *** WON auction for {intent_id}! amount_out={amount_out}, fee={fee}", cfg.tag());
    } else {
        println!("[{}] Lost auction for {intent_id} to {:?}", cfg.tag(), bid_solver.unwrap_or("unknown"));
    }
}

fn handle_execution_completed(cfg: &Config, data: &serde_json::Value) {
    let exec_data = data.get("data").unwrap_or(data);
    let solver_id = exec_data.get("solver_id").and_then(|s| s.as_str()).unwrap_or("");
    if solver_id != cfg.solver_id { return; }

    let intent_id = exec_data.get("intent_id").and_then(|s| s.as_str()).unwrap_or("unknown");
    let tx_hash = exec_data.get("tx_hash").and_then(|s| s.as_str()).unwrap_or("unknown");

    println!("[{}] *** Execution completed for {intent_id}", cfg.tag());
    println!("[{}]     tx_hash: {tx_hash}", cfg.tag());
    println!("[{}]     Profit realized!", cfg.tag());
}
