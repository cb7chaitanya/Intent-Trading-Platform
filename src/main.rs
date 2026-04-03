mod api;
mod db;
mod engine;
mod models;
mod services;
mod users;
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
use crate::users::repository::UserRepository;
use crate::users::service::UserService;
use crate::ws::server::WsServer;

const REDIS_URL: &str = "redis://127.0.0.1:6379";
const DATABASE_URL: &str = "postgres://postgres:postgres@127.0.0.1:5432/intent_trading";
const SERVER_ADDR: &str = "0.0.0.0:3000";

#[tokio::main]
async fn main() {
    println!("Starting Intent-Based Trading Platform...");

    // PostgreSQL connection pool
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(DATABASE_URL)
        .await
        .expect("Failed to connect to PostgreSQL");

    // Run migrations / ensure table exists
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id UUID PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create users table");

    // User service
    let user_repo = UserRepository::new(pg_pool);
    let user_service = Arc::new(UserService::new(user_repo));

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

    let app = api::router(app_state)
        .merge(ws_server.router())
        .merge(users::router(user_service));

    // Start server
    let listener = tokio::net::TcpListener::bind(SERVER_ADDR)
        .await
        .expect("Failed to bind server address");
    println!("Server listening on {SERVER_ADDR}");
    axum::serve(listener, app).await.expect("Server error");
}
