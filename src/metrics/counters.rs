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

pub static AUCTIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("auctions_total", "Total auctions completed").unwrap();
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

pub static CACHE_HITS: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("cache_hits_total", "Cache hits by key type"),
        &["key_type"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static CACHE_MISSES: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("cache_misses_total", "Cache misses by key type"),
        &["key_type"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static ETH_TX_SUBMITTED: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("eth_tx_submitted_total", "Ethereum transactions submitted"),
        &["outcome"], // "success", "nonce_retry", "replaced", "failed"
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static ETH_TX_RETRIES: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("eth_tx_retries_total", "Ethereum tx submission retries").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static DB_QUERIES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("db_queries_total", "Total database queries"),
        &["operation"],
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static HTLC_SWAPS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("htlc_swaps_total", "HTLC swaps by outcome"),
        &["outcome"], // "started", "completed", "refunded", "failed"
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static CROSS_CHAIN_LEGS_PROCESSED: Lazy<IntCounterVec> = Lazy::new(|| {
    let c = IntCounterVec::new(
        Opts::new("cross_chain_legs_processed_total", "Cross-chain legs processed by outcome"),
        &["outcome"], // "confirmed", "refunded", "failed", "executing"
    )
    .unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

pub static CROSS_CHAIN_TIMEOUTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("cross_chain_timeouts_total", "Cross-chain legs that timed out").unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});
