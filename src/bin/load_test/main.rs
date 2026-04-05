mod auctions;
mod intents;
mod metrics;
mod solvers;
mod users;

use std::sync::Arc;
use std::time::Duration;

use metrics::LoadMetrics;

struct Config {
    base_url: String,
    ws_url: String,
    num_users: u64,
    intents_per_second: u64,
    solvers_per_auction: u64,
    duration_secs: u64,
    ws_subscribers: u64,
    settlement_timeout_secs: u64,
    orderbook_queries_per_second: u64,
    /// Fake market ID used for orderbook queries when no real markets exist.
    orderbook_market_id: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            base_url: std::env::var("LOAD_TEST_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3000".into()),
            ws_url: std::env::var("LOAD_TEST_WS_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:3000/ws/feed".into()),
            num_users: parse_env("LOAD_TEST_USERS", 1000),
            intents_per_second: parse_env("LOAD_TEST_INTENTS_PER_SEC", 50),
            solvers_per_auction: parse_env("LOAD_TEST_SOLVERS", 5),
            duration_secs: parse_env("LOAD_TEST_DURATION_SECS", 60),
            ws_subscribers: parse_env("LOAD_TEST_WS_SUBS", 50),
            settlement_timeout_secs: parse_env("LOAD_TEST_SETTLEMENT_TIMEOUT", 30),
            orderbook_queries_per_second: parse_env("LOAD_TEST_OB_QPS", 20),
            orderbook_market_id: std::env::var("LOAD_TEST_MARKET_ID")
                .unwrap_or_else(|_| "00000000-0000-0000-0000-000000000000".into()),
        }
    }
}

fn parse_env(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let metrics = LoadMetrics::new();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(200)
        .build()
        .expect("Failed to build HTTP client");

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║              INTENTX LOAD TEST                          ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("  Target:          {}", config.base_url);
    println!("  Users:           {}", config.num_users);
    println!("  Intents/sec:     {}", config.intents_per_second);
    println!("  Solvers/auction: {}", config.solvers_per_auction);
    println!("  OB queries/sec:  {}", config.orderbook_queries_per_second);
    println!("  WS subscribers:  {}", config.ws_subscribers);
    println!("  Duration:        {}s", config.duration_secs);
    println!();

    // ── Phase 1: Create test users ───────────────────
    println!("[1/5] Creating {} test users...", config.num_users);
    let test_users = users::create_test_users(&client, &config.base_url, config.num_users).await;
    if test_users.is_empty() {
        eprintln!("ERROR: No test users created. Aborting.");
        return;
    }

    // ── Phase 2: Discover markets for orderbook queries ──
    println!("[2/5] Discovering markets...");
    let market_ids = discover_markets(&client, &config).await;
    println!("  Found {} markets", market_ids.len());

    // ── Phase 3: Start WebSocket subscribers ─────────
    println!(
        "[3/5] Starting {} WebSocket subscribers...",
        config.ws_subscribers
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut ws_handles = Vec::new();
    for _ in 0..config.ws_subscribers {
        let m = Arc::clone(&metrics);
        let rx = shutdown_rx.clone();
        let url = config.ws_url.clone();
        ws_handles.push(tokio::spawn(async move {
            auctions::ws_subscriber(&url, m, rx).await;
        }));
    }

    // ── Phase 4: Run load generation ─────────────────
    println!("[4/5] Generating load for {}s...", config.duration_secs);
    let intent_interval = Duration::from_secs_f64(1.0 / config.intents_per_second.max(1) as f64);
    let ob_interval = if config.orderbook_queries_per_second > 0 {
        Duration::from_secs_f64(1.0 / config.orderbook_queries_per_second as f64)
    } else {
        Duration::from_secs(3600) // effectively disabled
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.duration_secs);

    let mut intent_tick = tokio::time::interval(intent_interval);
    let mut ob_tick = tokio::time::interval(ob_interval);
    let mut user_idx: usize = 0;
    let mut settlement_handles = Vec::new();

    // Progress reporting
    let progress_metrics = Arc::clone(&metrics);
    let progress_duration = config.duration_secs;
    let progress_handle = tokio::spawn(async move {
        let interval = 10.max(progress_duration / 6);
        let mut tick = tokio::time::interval(Duration::from_secs(interval));
        tick.tick().await; // skip first
        loop {
            tick.tick().await;
            let elapsed = progress_metrics.elapsed_secs();
            if elapsed >= progress_duration as f64 {
                break;
            }
            let pct = (elapsed / progress_duration as f64 * 100.0).min(100.0);
            println!(
                "  [{:.0}%] {:.0}s elapsed | intents={} bids={} trades={} ws_msgs={}",
                pct,
                elapsed,
                progress_metrics
                    .intents_sent
                    .load(std::sync::atomic::Ordering::Relaxed),
                progress_metrics
                    .bids_sent
                    .load(std::sync::atomic::Ordering::Relaxed),
                progress_metrics
                    .trades_executed
                    .load(std::sync::atomic::Ordering::Relaxed),
                progress_metrics
                    .ws_messages
                    .load(std::sync::atomic::Ordering::Relaxed),
            );
        }
    });

    loop {
        tokio::select! {
            _ = intent_tick.tick() => {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }

                let user = Arc::clone(&test_users[user_idx % test_users.len()]);
                user_idx += 1;

                let c = client.clone();
                let m = Arc::clone(&metrics);
                let base = config.base_url.clone();
                let solver_count = config.solvers_per_auction;
                let timeout = config.settlement_timeout_secs;

                let handle = tokio::spawn(async move {
                    // Submit intent
                    let intent_id = intents::submit_random_intent(&c, &base, &user, &m).await;
                    let Some(intent_id) = intent_id else { return };

                    // Simulate concurrent solver bids
                    let mut bid_handles = Vec::new();
                    for s in 0..solver_count {
                        let c2 = c.clone();
                        let iid = intent_id.clone();
                        let m2 = Arc::clone(&m);
                        let b = base.clone();
                        bid_handles.push(tokio::spawn(async move {
                            solvers::submit_solver_bid(&c2, &b, &iid, s, &m2).await;
                        }));
                    }
                    for h in bid_handles {
                        let _ = h.await;
                    }

                    // Track settlement
                    auctions::track_settlement(&c, &base, &intent_id, &user.token, &m, timeout).await;
                });

                settlement_handles.push(handle);
            }
            _ = ob_tick.tick() => {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                if !market_ids.is_empty() {
                    let mid = market_ids[user_idx % market_ids.len()].clone();
                    let c = client.clone();
                    let m = Arc::clone(&metrics);
                    let base = config.base_url.clone();
                    tokio::spawn(async move {
                        auctions::query_orderbook(&c, &base, &mid, &m).await;
                    });
                }
            }
        }
    }

    // ── Phase 5: Drain and report ────────────────────
    println!("[5/5] Waiting for in-flight settlements...");
    let wait_deadline =
        tokio::time::Instant::now() + Duration::from_secs(config.settlement_timeout_secs + 10);

    for handle in settlement_handles {
        tokio::select! {
            _ = handle => {}
            _ = tokio::time::sleep_until(wait_deadline) => {
                println!("  Timed out waiting for settlements");
                break;
            }
        }
    }

    // Shutdown WS subscribers and progress reporter
    let _ = shutdown_tx.send(true);
    for h in ws_handles {
        let _ = h.await;
    }
    progress_handle.abort();

    // Scrape server-side metrics
    println!();
    println!("  Scraping server metrics...");
    let server_metrics = metrics::scrape_server_metrics(&config.base_url).await;

    // Final report
    metrics.report(server_metrics.as_ref()).await;
}

async fn discover_markets(client: &reqwest::Client, config: &Config) -> Vec<String> {
    let resp = client
        .get(format!("{}/markets", config.base_url))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let markets: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            let ids: Vec<String> = markets
                .iter()
                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                .collect();
            if ids.is_empty() {
                vec![config.orderbook_market_id.clone()]
            } else {
                ids
            }
        }
        _ => vec![config.orderbook_market_id.clone()],
    }
}
