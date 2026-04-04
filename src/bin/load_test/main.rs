mod auctions;
mod intents;
mod metrics;
mod solvers;
mod users;

use std::sync::Arc;
use std::time::Duration;

use metrics::LoadMetrics;

const BASE_URL: &str = "http://127.0.0.1:3000";
const WS_URL: &str = "ws://127.0.0.1:3000/ws/feed";

struct Config {
    num_users: u64,
    intents_per_second: u64,
    solvers_per_auction: u64,
    duration_secs: u64,
    ws_subscribers: u64,
    settlement_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num_users: 10,
            intents_per_second: 5,
            solvers_per_auction: 3,
            duration_secs: 30,
            ws_subscribers: 5,
            settlement_timeout_secs: 30,
        }
    }
}

#[tokio::main]
async fn main() {
    let config = Config::default();
    let metrics = LoadMetrics::new();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");

    println!("=== IntentX Load Test ===");
    println!(
        "  Users: {}  Intents/s: {}  Solvers/auction: {}  Duration: {}s",
        config.num_users,
        config.intents_per_second,
        config.solvers_per_auction,
        config.duration_secs,
    );
    println!();

    // Phase 1: Create test users
    println!("[1/4] Creating test users...");
    let test_users = users::create_test_users(&client, BASE_URL, config.num_users).await;

    // Phase 2: Start WS subscribers
    println!("[2/4] Starting {} WebSocket subscribers...", config.ws_subscribers);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut ws_handles = Vec::new();
    for _ in 0..config.ws_subscribers {
        let m = Arc::clone(&metrics);
        let rx = shutdown_rx.clone();
        ws_handles.push(tokio::spawn(async move {
            auctions::ws_subscriber(WS_URL, m, rx).await;
        }));
    }

    // Phase 3: Run load generation
    println!(
        "[3/4] Generating load for {}s...",
        config.duration_secs
    );
    let interval = Duration::from_secs_f64(1.0 / config.intents_per_second as f64);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.duration_secs);

    let mut intent_interval = tokio::time::interval(interval);
    let mut user_idx: usize = 0;
    let mut settlement_handles = Vec::new();

    loop {
        tokio::select! {
            _ = intent_interval.tick() => {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }

                let user = Arc::clone(&test_users[user_idx % test_users.len()]);
                user_idx += 1;

                let c = client.clone();
                let m = Arc::clone(&metrics);
                let solvers = config.solvers_per_auction;
                let timeout = config.settlement_timeout_secs;

                let handle = tokio::spawn(async move {
                    // Submit intent
                    let intent_id = intents::submit_random_intent(
                        &c, BASE_URL, &user, &m,
                    ).await;

                    let Some(intent_id) = intent_id else { return };

                    // Simulate solvers bidding
                    let mut bid_handles = Vec::new();
                    for s in 0..solvers {
                        let c2 = c.clone();
                        let iid = intent_id.clone();
                        let m2 = Arc::clone(&m);
                        bid_handles.push(tokio::spawn(async move {
                            solvers::submit_solver_bid(&c2, BASE_URL, &iid, s, &m2).await;
                        }));
                    }
                    for h in bid_handles {
                        let _ = h.await;
                    }

                    // Track settlement
                    auctions::track_settlements(&c, BASE_URL, &intent_id, &m, timeout).await;
                });

                settlement_handles.push(handle);
            }
        }
    }

    // Wait for in-flight work
    println!("[4/4] Waiting for in-flight settlements...");
    let wait_deadline = tokio::time::Instant::now()
        + Duration::from_secs(config.settlement_timeout_secs + 5);

    for handle in settlement_handles {
        tokio::select! {
            _ = handle => {}
            _ = tokio::time::sleep_until(wait_deadline) => {
                println!("  Timed out waiting for settlements");
                break;
            }
        }
    }

    // Shutdown WS subscribers
    let _ = shutdown_tx.send(true);
    for h in ws_handles {
        let _ = h.await;
    }

    // Report
    metrics.report().await;
}
