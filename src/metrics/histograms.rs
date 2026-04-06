use once_cell::sync::Lazy;
use prometheus::{Histogram, HistogramOpts, HistogramVec};

use super::REGISTRY;

pub static API_REQUEST_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "api_request_duration_seconds",
            "API request latency in seconds",
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5]),
        &["method", "endpoint"],
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static MATCHING_ENGINE_LATENCY: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "matching_engine_latency_seconds",
            "Time to match an intent with best bid",
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static AUCTION_DURATION: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "auction_duration_seconds",
            "Duration of auction from start to close",
        )
        .buckets(vec![1.0, 2.0, 5.0, 10.0, 15.0, 20.0, 30.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static TRADE_EXECUTION_DURATION: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "trade_execution_duration_seconds",
            "Time from execution start to completion",
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static DB_QUERY_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "db_query_duration_seconds",
            "Database query latency in seconds",
        )
        .buckets(vec![0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5]),
        &["operation"],
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static HTLC_SWAP_DURATION: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "htlc_swap_duration_seconds",
            "End-to-end HTLC swap time (created to completed/refunded)",
        )
        .buckets(vec![10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

pub static SETTLEMENT_DURATION: Lazy<Histogram> = Lazy::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new(
            "settlement_duration_seconds",
            "Time to settle a trade",
        )
        .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});
