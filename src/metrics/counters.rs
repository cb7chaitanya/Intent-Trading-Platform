use once_cell::sync::Lazy;
use prometheus::{IntCounter, IntCounterVec, Opts};

use super::REGISTRY;

pub static INTENTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("intents_total", "Total intents submitted").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static TRADES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("trades_total", "Total trades executed").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static BIDS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("bids_total", "Total bids submitted").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static SETTLEMENT_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("settlement_success_total", "Total successful settlements").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static SETTLEMENT_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("settlement_failures_total", "Total failed settlements").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static API_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("api_requests_total", "Total API requests per endpoint"),
        &["method", "endpoint", "status"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static FEES_COLLECTED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("fees_collected_total", "Total fees collected (base units)").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static TRADE_VOLUME: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("trade_volume_total", "Trade volume per market"),
        &["market_id"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static SOLVER_WINS: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("solver_wins_total", "Auction wins per solver"),
        &["solver_id"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static TRADES_PER_SECOND: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("trades_executed_counter", "Monotonic trade counter for rate calculation").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});
