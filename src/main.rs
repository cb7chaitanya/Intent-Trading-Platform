mod accounts;
mod api;
mod balances;
mod db;
mod ledger;
mod engine;
mod models;
mod services;
mod settlement;
mod users;
mod ws;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::accounts::repository::AccountRepository;
use crate::accounts::service::AccountService;
use crate::balances::repository::BalanceRepository;
use crate::balances::service::BalanceService;
use crate::ledger::repository::LedgerRepository;
use crate::ledger::service::LedgerService;
use crate::settlement::engine::SettlementEngine;
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

    sqlx::query(
        "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'account_type') THEN
                CREATE TYPE account_type AS ENUM ('spot');
            END IF;
        END $$",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create account_type enum");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS accounts (
            id UUID PRIMARY KEY,
            user_id UUID NOT NULL REFERENCES users(id),
            account_type account_type NOT NULL DEFAULT 'spot',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create accounts table");

    sqlx::query(
        "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'asset_type') THEN
                CREATE TYPE asset_type AS ENUM ('USDC', 'ETH', 'BTC', 'SOL');
            END IF;
        END $$",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create asset_type enum");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS balances (
            id UUID PRIMARY KEY,
            account_id UUID NOT NULL REFERENCES accounts(id),
            asset asset_type NOT NULL,
            available_balance BIGINT NOT NULL DEFAULT 0,
            locked_balance BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (account_id, asset)
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create balances table");

    sqlx::query(
        "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'entry_type') THEN
                CREATE TYPE entry_type AS ENUM ('DEBIT', 'CREDIT');
            END IF;
        END $$",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create entry_type enum");

    sqlx::query(
        "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'reference_type') THEN
                CREATE TYPE reference_type AS ENUM ('TRADE', 'DEPOSIT', 'WITHDRAWAL', 'FEE');
            END IF;
        END $$",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create reference_type enum");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ledger_entries (
            id UUID PRIMARY KEY,
            account_id UUID NOT NULL REFERENCES accounts(id),
            asset asset_type NOT NULL,
            amount BIGINT NOT NULL,
            entry_type entry_type NOT NULL,
            reference_type reference_type NOT NULL,
            reference_id UUID NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create ledger_entries table");

    // Account service
    let account_repo = AccountRepository::new(pg_pool.clone());
    let account_service = Arc::new(AccountService::new(account_repo));

    // Ledger service
    let ledger_repo = LedgerRepository::new(pg_pool.clone());
    let ledger_service = Arc::new(LedgerService::new(ledger_repo));

    // Balance service
    let balance_repo = BalanceRepository::new(pg_pool.clone());
    let balance_service = Arc::new(BalanceService::new(balance_repo, Arc::clone(&ledger_service)));

    sqlx::query(
        "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'trade_status') THEN
                CREATE TYPE trade_status AS ENUM ('pending', 'settled', 'failed');
            END IF;
        END $$",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create trade_status enum");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS trades (
            id UUID PRIMARY KEY,
            buyer_account_id UUID NOT NULL REFERENCES accounts(id),
            seller_account_id UUID NOT NULL REFERENCES accounts(id),
            solver_account_id UUID NOT NULL REFERENCES accounts(id),
            asset_in asset_type NOT NULL,
            asset_out asset_type NOT NULL,
            amount_in BIGINT NOT NULL,
            amount_out BIGINT NOT NULL,
            platform_fee BIGINT NOT NULL DEFAULT 0,
            solver_fee BIGINT NOT NULL DEFAULT 0,
            status trade_status NOT NULL DEFAULT 'pending',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            settled_at TIMESTAMPTZ
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create trades table");

    // Settlement engine
    let settlement_engine = Arc::new(SettlementEngine::new(pg_pool.clone()));

    // User service
    let user_repo = UserRepository::new(pg_pool);
    let user_service = Arc::new(UserService::new(user_repo, Arc::clone(&account_service)));

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
        .merge(users::router(user_service))
        .merge(accounts::router(account_service))
        .merge(balances::router(balance_service))
        .merge(ledger::router(ledger_service))
        .merge(settlement::router(settlement_engine));

    // Start server
    let listener = tokio::net::TcpListener::bind(SERVER_ADDR)
        .await
        .expect("Failed to bind server address");
    println!("Server listening on {SERVER_ADDR}");
    axum::serve(listener, app).await.expect("Server error");
}
