use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

#[derive(Default)]
struct Inner {
    latencies_intent: Vec<f64>,
    latencies_bid: Vec<f64>,
    latencies_settlement: Vec<f64>,
    latencies_ws: Vec<f64>,
}

pub struct LoadMetrics {
    pub intents_sent: AtomicU64,
    pub intents_failed: AtomicU64,
    pub bids_sent: AtomicU64,
    pub bids_failed: AtomicU64,
    pub trades_executed: AtomicU64,
    pub settlements_ok: AtomicU64,
    pub settlements_failed: AtomicU64,
    pub ws_messages: AtomicU64,
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
            ws_messages: AtomicU64::new(0),
            start: Instant::now(),
            inner: Mutex::new(Inner::default()),
        })
    }

    pub async fn record_intent_latency(&self, ms: f64) {
        self.inner.lock().await.latencies_intent.push(ms);
    }

    pub async fn record_bid_latency(&self, ms: f64) {
        self.inner.lock().await.latencies_bid.push(ms);
    }

    pub async fn record_settlement_latency(&self, ms: f64) {
        self.inner.lock().await.latencies_settlement.push(ms);
    }

    pub async fn record_ws_latency(&self, ms: f64) {
        self.inner.lock().await.latencies_ws.push(ms);
    }

    pub async fn report(&self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let intents = self.intents_sent.load(Ordering::Relaxed);
        let bids = self.bids_sent.load(Ordering::Relaxed);
        let trades = self.trades_executed.load(Ordering::Relaxed);

        let inner = self.inner.lock().await;

        println!("\n{}", "=".repeat(60));
        println!("  LOAD TEST RESULTS");
        println!("{}", "=".repeat(60));
        println!("  Duration:              {:.1}s", elapsed);
        println!();

        println!("  Throughput:");
        println!("    Intents/sec:         {:.1}", intents as f64 / elapsed);
        println!("    Bids/sec:            {:.1}", bids as f64 / elapsed);
        println!("    Trades/sec:          {:.1}", trades as f64 / elapsed);
        println!();

        println!("  Totals:");
        println!(
            "    Intents:             {} sent, {} failed",
            intents,
            self.intents_failed.load(Ordering::Relaxed)
        );
        println!(
            "    Bids:                {} sent, {} failed",
            bids,
            self.bids_failed.load(Ordering::Relaxed)
        );
        println!("    Trades:              {}", trades);
        println!(
            "    Settlements:         {} ok, {} failed",
            self.settlements_ok.load(Ordering::Relaxed),
            self.settlements_failed.load(Ordering::Relaxed),
        );
        println!(
            "    WS messages:         {}",
            self.ws_messages.load(Ordering::Relaxed)
        );
        println!();

        println!("  Latency (ms):");
        print_latency("    Intent API", &inner.latencies_intent);
        print_latency("    Bid API", &inner.latencies_bid);
        print_latency("    Settlement", &inner.latencies_settlement);
        print_latency("    WS roundtrip", &inner.latencies_ws);
        println!();
    }
}

fn print_latency(label: &str, values: &[f64]) {
    if values.is_empty() {
        println!("{label}:     (no data)");
        return;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() as f64 * 0.95) as usize];
    let p99 = sorted[(sorted.len() as f64 * 0.99).min(sorted.len() as f64 - 1.0) as usize];
    println!(
        "{label}:     avg={avg:.1}  p50={p50:.1}  p95={p95:.1}  p99={p99:.1}"
    );
}
