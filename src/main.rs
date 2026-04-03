mod api;
mod db;
mod engine;
mod models;
mod services;
mod ws;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::api::AppState;
use crate::db::redis::EventBus;
use crate::db::storage::Storage;
use crate::engine::auction_engine::AuctionEngine;
use crate::engine::execution_engine::ExecutionEngine;
use crate::services::bid_service::BidService;
use crate::services::intent_service::IntentService;
use crate::ws::server::WsServer;

const REDIS_URL: &str = "redis://127.0.0.1:6379";
const SERVER_ADDR: &str = "0.0.0.0:3000";

#[tokio::main]
async fn main() {
    println!("Starting Intent-Based Trading Platform...");

    // Shared storage
    let storage = Arc::new(Storage::new());

    // Each component gets its own EventBus (separate Redis connections)
    let intent_bus = EventBus::new(REDIS_URL).await.expect("Failed to connect Redis for IntentService");
    let bid_bus = EventBus::new(REDIS_URL).await.expect("Failed to connect Redis for BidService");
    let auction_bus = EventBus::new(REDIS_URL).await.expect("Failed to connect Redis for AuctionEngine");
    let execution_bus = EventBus::new(REDIS_URL).await.expect("Failed to connect Redis for ExecutionEngine");
    let ws_bus = EventBus::new(REDIS_URL).await.expect("Failed to connect Redis for WsServer");

    // Services
    let intent_service = Arc::new(Mutex::new(IntentService::new(Arc::clone(&storage), intent_bus)));
    let bid_service = Arc::new(Mutex::new(BidService::new(Arc::clone(&storage), bid_bus)));

    // Engines
    let auction_engine = AuctionEngine::new(Arc::clone(&storage), auction_bus);
    let execution_engine = ExecutionEngine::new(Arc::clone(&storage), execution_bus);

    // WebSocket server
    let ws_server = WsServer::new();
    let ws_redis_client = ws_bus.client().clone();

    // Spawn background tasks
    tokio::spawn(async move {
        if let Err(e) = auction_engine.start().await {
            eprintln!("AuctionEngine error: {e}");
        }
    });

    tokio::spawn(async move {
        if let Err(e) = execution_engine.start().await {
            eprintln!("ExecutionEngine error: {e}");
        }
    });

    let ws_listener = ws_server.clone();
    tokio::spawn(async move {
        ws_listener.start_redis_listener(&ws_redis_client).await;
    });

    // Build combined router: API + WebSocket
    let app_state = AppState {
        intent_service,
        bid_service,
    };

    let app = api::router(app_state).merge(ws_server.router());

    // Start server
    let listener = tokio::net::TcpListener::bind(SERVER_ADDR)
        .await
        .expect("Failed to bind server address");
    println!("Server listening on {SERVER_ADDR}");
    axum::serve(listener, app).await.expect("Server error");
}
