use once_cell::sync::Lazy;
use prometheus::IntGauge;

use super::REGISTRY;

pub static ACTIVE_AUCTIONS: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new("active_auctions", "Number of currently running auctions").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

pub static WEBSOCKET_CONNECTIONS: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new("websocket_connections", "Active WebSocket connections").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

pub static BIDS_PER_AUCTION: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new("bids_per_auction_latest", "Bids received in latest auction").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

pub static CROSS_CHAIN_PENDING_LEGS: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new("cross_chain_pending_legs", "Cross-chain legs in non-terminal state").unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});
