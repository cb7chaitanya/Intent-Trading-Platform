use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const SERVER_URL: &str = "http://127.0.0.1:3000";
const WS_URL: &str = "ws://127.0.0.1:3000/ws";
const SOLVER_ID: &str = "solver-bot-alpha";

#[derive(Debug, Deserialize)]
struct WsEvent {
    event: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Intent {
    id: Uuid,
    user_id: String,
    token_in: String,
    token_out: String,
    amount_in: u64,
    min_amount_out: u64,
    deadline: i64,
    status: String,
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
    intent_id: Uuid,
    solver_id: String,
    amount_out: u64,
    fee: u64,
    timestamp: i64,
}

#[tokio::main]
async fn main() {
    println!("[{SOLVER_ID}] Starting solver bot...");

    loop {
        match run().await {
            Ok(()) => {
                println!("[{SOLVER_ID}] WebSocket closed, reconnecting...");
            }
            Err(e) => {
                eprintln!("[{SOLVER_ID}] Error: {e}, reconnecting in 3s...");
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("[{SOLVER_ID}] Connecting to {WS_URL}...");
    let (ws_stream, _) = connect_async(WS_URL).await?;
    println!("[{SOLVER_ID}] Connected to WebSocket");

    let (mut _write, mut read) = ws_stream.split();

    let http = reqwest::Client::new();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                eprintln!("[{SOLVER_ID}] WS read error: {e}");
                break;
            }
        };

        let ws_event: WsEvent = match serde_json::from_str(&msg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[{SOLVER_ID}] Failed to parse event: {e}");
                continue;
            }
        };

        match ws_event.event.as_str() {
            "intent_created" => {
                handle_new_intent(&http, &ws_event.data).await;
            }
            "intent_matched" => {
                handle_matched(&ws_event.data);
            }
            "execution_completed" => {
                handle_execution_completed(&ws_event.data);
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_new_intent(http: &reqwest::Client, data: &serde_json::Value) {
    // The Event is tagged: {"type": "IntentCreated", "data": {...}}
    let intent_value = match data.get("data") {
        Some(v) => v,
        None => {
            eprintln!("[{SOLVER_ID}] Missing intent data in event");
            return;
        }
    };

    let intent: Intent = match serde_json::from_value(intent_value.clone()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[{SOLVER_ID}] Failed to parse intent: {e}");
            return;
        }
    };

    println!(
        "[{SOLVER_ID}] New intent detected: {} | {} -> {} | amount: {}",
        intent.id, intent.token_in, intent.token_out, intent.amount_in
    );

    // Generate quote: offer between min_amount_out and 110% of min_amount_out
    let mut rng = rand::rng();
    let premium: f64 = rng.random_range(1.0..1.1);
    let amount_out = (intent.min_amount_out as f64 * premium) as u64;
    let fee = intent.amount_in / 200; // 0.5% fee

    let profit = amount_out.saturating_sub(intent.min_amount_out).saturating_sub(fee);
    println!(
        "[{SOLVER_ID}] Quoting: amount_out={amount_out}, fee={fee}, estimated_profit={profit}"
    );

    let bid = BidRequest {
        intent_id: intent.id,
        solver_id: SOLVER_ID.to_string(),
        amount_out,
        fee,
    };

    match http
        .post(format!("{SERVER_URL}/bids"))
        .json(&bid)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<BidResponse>().await {
                Ok(bid_resp) => {
                    println!(
                        "[{SOLVER_ID}] Bid submitted: id={}, amount_out={}, fee={}",
                        bid_resp.id, bid_resp.amount_out, bid_resp.fee
                    );
                }
                Err(e) => eprintln!("[{SOLVER_ID}] Failed to parse bid response: {e}"),
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            eprintln!("[{SOLVER_ID}] Bid rejected: {status} - {body}");
        }
        Err(e) => {
            eprintln!("[{SOLVER_ID}] Failed to submit bid: {e}");
        }
    }
}

fn handle_matched(data: &serde_json::Value) {
    // {"type": "IntentMatched", "data": {"intent": {...}, "bid": {...}}}
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

    if bid_solver == Some(SOLVER_ID) {
        let amount_out = matched_data
            .get("bid")
            .and_then(|b| b.get("amount_out"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let fee = matched_data
            .get("bid")
            .and_then(|b| b.get("fee"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        println!("[{SOLVER_ID}] *** WON auction for intent {intent_id}! amount_out={amount_out}, fee={fee}");
    } else {
        println!(
            "[{SOLVER_ID}] Lost auction for intent {intent_id} to {:?}",
            bid_solver.unwrap_or("unknown")
        );
    }
}

fn handle_execution_completed(data: &serde_json::Value) {
    let exec_data = data.get("data").unwrap_or(data);

    let solver_id = exec_data
        .get("solver_id")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");

    if solver_id != SOLVER_ID {
        return;
    }

    let intent_id = exec_data
        .get("intent_id")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");
    let tx_hash = exec_data
        .get("tx_hash")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");

    println!("[{SOLVER_ID}] *** Execution completed for intent {intent_id}");
    println!("[{SOLVER_ID}]     tx_hash: {tx_hash}");
    println!("[{SOLVER_ID}]     Profit realized!");
}
