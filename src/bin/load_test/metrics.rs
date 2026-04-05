use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

// ── Latency bucket ───────────────────────────────────────

#[derive(Default)]
pub struct LatencyBucket {
    samples: Vec<f64>,
}

impl LatencyBucket {
    pub fn push(&mut self, ms: f64) {
        self.samples.push(ms);
    }

    pub fn count(&self) -> usize {
        self.samples.len()
    }

    pub fn percentile(&self, pct: f64) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() as f64 * pct / 100.0) as usize).min(sorted.len() - 1);
        sorted[idx]
    }

    pub fn avg(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / self.samples.len() as f64
    }

    pub fn min(&self) -> f64 {
        self.samples.iter().cloned().fold(f64::INFINITY, f64::min)
    }

    pub fn max(&self) -> f64 {
        self.samples
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

// ── Inner state (mutex-protected) ────────────────────────

#[derive(Default)]
struct Inner {
    lat_intent: LatencyBucket,
    lat_bid: LatencyBucket,
    lat_settlement: LatencyBucket,
    lat_orderbook: LatencyBucket,
    lat_ws: LatencyBucket,
}

// ── Public metrics ───────────────────────────────────────

pub struct LoadMetrics {
    pub intents_sent: AtomicU64,
    pub intents_failed: AtomicU64,
    pub bids_sent: AtomicU64,
    pub bids_failed: AtomicU64,
    pub trades_executed: AtomicU64,
    pub settlements_ok: AtomicU64,
    pub settlements_failed: AtomicU64,
    pub orderbook_queries: AtomicU64,
    pub orderbook_failed: AtomicU64,
    pub ws_messages: AtomicU64,
    pub ws_connections: AtomicU64,
    pub ws_errors: AtomicU64,
    start: Instant,
    inner: Mutex<Inner>,
}

impl LoadMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            intents_sent: AtomicU64::new(0),
            intents_failed: AtomicU64::new(0),
            bids_sent: AtomicU64::new(0),
            bids_failed: AtomicU64::new(0),
            trades_executed: AtomicU64::new(0),
            settlements_ok: AtomicU64::new(0),
            settlements_failed: AtomicU64::new(0),
            orderbook_queries: AtomicU64::new(0),
            orderbook_failed: AtomicU64::new(0),
            ws_messages: AtomicU64::new(0),
            ws_connections: AtomicU64::new(0),
            ws_errors: AtomicU64::new(0),
            start: Instant::now(),
            inner: Mutex::new(Inner::default()),
        })
    }

    pub async fn record_intent_latency(&self, ms: f64) {
        self.inner.lock().await.lat_intent.push(ms);
    }

    pub async fn record_bid_latency(&self, ms: f64) {
        self.inner.lock().await.lat_bid.push(ms);
    }

    pub async fn record_settlement_latency(&self, ms: f64) {
        self.inner.lock().await.lat_settlement.push(ms);
    }

    pub async fn record_orderbook_latency(&self, ms: f64) {
        self.inner.lock().await.lat_orderbook.push(ms);
    }

    pub async fn record_ws_latency(&self, ms: f64) {
        self.inner.lock().await.lat_ws.push(ms);
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Print the full benchmark report to stdout.
    pub async fn report(&self, server_metrics: Option<&ServerMetrics>) {
        let elapsed = self.elapsed_secs();
        let intents = self.intents_sent.load(Ordering::Relaxed);
        let bids = self.bids_sent.load(Ordering::Relaxed);
        let trades = self.trades_executed.load(Ordering::Relaxed);
        let ob_queries = self.orderbook_queries.load(Ordering::Relaxed);
        let total_requests = intents + bids + ob_queries;

        let inner = self.inner.lock().await;

        println!();
        println!("{}", "═".repeat(70));
        println!("  INTENTX LOAD TEST — BENCHMARK REPORT");
        println!("{}", "═".repeat(70));
        println!();

        // ── Summary ──────────────────────────────────
        println!("  Duration:                  {:.1}s", elapsed);
        println!("  Total requests:            {}", total_requests);
        println!(
            "  Requests/sec:              {:.1}",
            total_requests as f64 / elapsed
        );
        println!();

        // ── Throughput ───────────────────────────────
        println!("  ┌─ Throughput ──────────────────────────────────────────┐");
        println!(
            "  │  Intents/sec:            {:<10.1}                   │",
            intents as f64 / elapsed
        );
        println!(
            "  │  Bids/sec:               {:<10.1}                   │",
            bids as f64 / elapsed
        );
        println!(
            "  │  Trades/sec:             {:<10.1}                   │",
            trades as f64 / elapsed
        );
        println!(
            "  │  Orderbook queries/sec:  {:<10.1}                   │",
            ob_queries as f64 / elapsed
        );
        println!("  └────────────────────────────────────────────────────────┘");
        println!();

        // ── Totals ───────────────────────────────────
        println!("  ┌─ Totals ─────────────────────────────────────────────┐");
        println!(
            "  │  Intents:       {} sent, {} failed                   ",
            intents,
            self.intents_failed.load(Ordering::Relaxed)
        );
        println!(
            "  │  Bids:          {} sent, {} failed                   ",
            bids,
            self.bids_failed.load(Ordering::Relaxed)
        );
        println!("  │  Trades:        {}                                  ", trades);
        println!(
            "  │  Settlements:   {} ok, {} failed                    ",
            self.settlements_ok.load(Ordering::Relaxed),
            self.settlements_failed.load(Ordering::Relaxed),
        );
        println!(
            "  │  Orderbook:     {} ok, {} failed                    ",
            ob_queries,
            self.orderbook_failed.load(Ordering::Relaxed)
        );
        println!(
            "  │  WebSocket:     {} msgs, {} conns, {} errors        ",
            self.ws_messages.load(Ordering::Relaxed),
            self.ws_connections.load(Ordering::Relaxed),
            self.ws_errors.load(Ordering::Relaxed),
        );
        println!("  └────────────────────────────────────────────────────────┘");
        println!();

        // ── Latency ──────────────────────────────────
        println!("  ┌─ Latency (ms) ───────────────────────────────────────┐");
        println!(
            "  │  {:20}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6} │",
            "", "avg", "p50", "p95", "p99", "max"
        );
        println!("  │  {}", "─".repeat(56));
        print_row("Intent API", &inner.lat_intent);
        print_row("Bid API", &inner.lat_bid);
        print_row("Orderbook API", &inner.lat_orderbook);
        print_row("Settlement E2E", &inner.lat_settlement);
        print_row("WebSocket msg", &inner.lat_ws);
        println!("  └────────────────────────────────────────────────────────┘");

        // ── Server-side metrics (scraped from /metrics) ──
        if let Some(sm) = server_metrics {
            println!();
            println!("  ┌─ Server Metrics (from /metrics endpoint) ────────────┐");
            if let Some(v) = sm.api_latency_avg_ms {
                println!("  │  API avg latency:        {:.1} ms                     │", v);
            }
            if let Some(v) = sm.db_query_avg_ms {
                println!("  │  DB query avg latency:   {:.1} ms                     │", v);
            }
            if let Some(v) = sm.settlement_avg_ms {
                println!("  │  Settlement avg latency: {:.1} ms                     │", v);
            }
            if let Some(v) = sm.auction_avg_secs {
                println!("  │  Auction avg duration:   {:.2} s                      │", v);
            }
            if let Some(v) = sm.active_auctions {
                println!("  │  Active auctions:        {}                           │", v);
            }
            if let Some(v) = sm.ws_connections {
                println!("  │  WebSocket connections:  {}                           │", v);
            }
            if let Some(v) = sm.cache_hit_rate {
                println!("  │  Cache hit rate:         {:.1}%                       │", v);
            }
            println!("  └────────────────────────────────────────────────────────┘");
        }

        println!();
        println!("{}", "═".repeat(70));
    }
}

fn print_row(label: &str, bucket: &LatencyBucket) {
    if bucket.count() == 0 {
        println!("  │  {:20}  {:>42} │", label, "(no data)");
        return;
    }
    println!(
        "  │  {:20}  {:6.1}  {:6.1}  {:6.1}  {:6.1}  {:6.1} │",
        label,
        bucket.avg(),
        bucket.percentile(50.0),
        bucket.percentile(95.0),
        bucket.percentile(99.0),
        bucket.max(),
    );
}

// ── Server-side metric scraping ──────────────────────────

#[derive(Debug, Default)]
pub struct ServerMetrics {
    pub api_latency_avg_ms: Option<f64>,
    pub db_query_avg_ms: Option<f64>,
    pub settlement_avg_ms: Option<f64>,
    pub auction_avg_secs: Option<f64>,
    pub active_auctions: Option<u64>,
    pub ws_connections: Option<u64>,
    pub cache_hit_rate: Option<f64>,
}

pub async fn scrape_server_metrics(base_url: &str) -> Option<ServerMetrics> {
    let client = reqwest::Client::new();
    let raw = client
        .get(format!("{base_url}/metrics"))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    let mut sm = ServerMetrics::default();

    sm.api_latency_avg_ms = parse_avg_ms(&raw, "api_request_duration_seconds");
    sm.db_query_avg_ms = parse_avg_ms(&raw, "db_query_duration_seconds");
    sm.settlement_avg_ms = parse_avg_ms(&raw, "settlement_duration_seconds");
    sm.auction_avg_secs = parse_avg(&raw, "auction_duration_seconds");
    sm.active_auctions = parse_gauge(&raw, "active_auctions");
    sm.ws_connections = parse_gauge(&raw, "websocket_connections");

    let hits = parse_sum_counter(&raw, "cache_hits_total");
    let misses = parse_sum_counter(&raw, "cache_misses_total");
    if hits + misses > 0.0 {
        sm.cache_hit_rate = Some(hits / (hits + misses) * 100.0);
    }

    Some(sm)
}

fn parse_sum_counter(raw: &str, name: &str) -> f64 {
    let re = regex_lite(name);
    let mut total = 0.0;
    for line in raw.lines() {
        if line.starts_with(name) && !line.starts_with('#') {
            if let Some(val) = line.rsplit_once(' ').and_then(|(_, v)| v.parse::<f64>().ok()) {
                total += val;
            }
        }
    }
    let _ = re; // keep compiler happy
    total
}

fn parse_gauge(raw: &str, name: &str) -> Option<u64> {
    for line in raw.lines() {
        if line.starts_with(name) && !line.starts_with('#') {
            return line
                .rsplit_once(' ')
                .and_then(|(_, v)| v.parse::<f64>().ok())
                .map(|v| v as u64);
        }
    }
    None
}

fn parse_avg_ms(raw: &str, name: &str) -> Option<f64> {
    parse_avg(raw, name).map(|s| s * 1000.0)
}

fn parse_avg(raw: &str, name: &str) -> Option<f64> {
    let sum_key = format!("{name}_sum");
    let count_key = format!("{name}_count");
    let mut sum = 0.0f64;
    let mut count = 0.0f64;

    for line in raw.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with(&sum_key) {
            if let Some(v) = line.rsplit_once(' ').and_then(|(_, v)| v.parse::<f64>().ok()) {
                sum += v;
            }
        }
        if line.starts_with(&count_key) {
            if let Some(v) = line.rsplit_once(' ').and_then(|(_, v)| v.parse::<f64>().ok()) {
                count += v;
            }
        }
    }

    if count > 0.0 {
        Some(sum / count)
    } else {
        None
    }
}

fn regex_lite(_: &str) -> () {} // placeholder to avoid unused import
