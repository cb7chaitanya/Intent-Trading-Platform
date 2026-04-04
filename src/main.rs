mod accounts;
mod api;
mod balances;
mod config;
mod db;
mod ledger;
mod engine;
mod fees;
mod market_data;
mod markets;
mod metrics;
mod models;
mod risk;
mod services;
mod settlement;
mod solver_reputation;
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
use crate::market_data::repository::MarketDataRepository;
use crate::market_data::service::MarketDataService;
use crate::markets::repository::MarketRepository;
use crate::markets::service::MarketService;
use crate::risk::service::RiskEngine;
use crate::settlement::engine::SettlementEngine;
use crate::solver_reputation::repository::SolverRepository;
use crate::solver_reputation::service::SolverReputationService;
use crate::api::AppState;
use crate::db::redis::EventBus;
use crate::db::stream_bus::StreamBus;
use crate::db::storage::Storage;
use crate::engine::auction_engine::AuctionEngine;
use crate::engine::execution_engine::ExecutionEngine;
use crate::services::bid_service::BidService;
use crate::services::intent_service::IntentService;
use crate::users::repository::UserRepository;
use crate::users::service::UserService;
use crate::ws::feed::WsFeed;
use crate::ws::server::WsServer;

#[tokio::main]
async fn main() {
    let cfg = config::init();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    format!(
                        "intent_trading={lvl},tower_http=info,sqlx=warn",
                        lvl = cfg.log_level
                    )
                    .into()
                }),
        )
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .init();

    tracing::info!(environment = %cfg.environment, "Starting Intent-Based Trading Platform");

    // Initialize all Prometheus metrics
    metrics::init();

    // PostgreSQL connection pool
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.pg_max_connections)
        .connect(&cfg.database_url)
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

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS markets (
            id UUID PRIMARY KEY,
            base_asset asset_type NOT NULL,
            quote_asset asset_type NOT NULL,
            tick_size BIGINT NOT NULL,
            min_order_size BIGINT NOT NULL,
            fee_rate DOUBLE PRECISION NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (base_asset, quote_asset)
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create markets table");

    // Market service
    let market_repo = MarketRepository::new(pg_pool.clone());
    let market_service = Arc::new(MarketService::new(market_repo));

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS market_trades (
            id UUID PRIMARY KEY,
            market_id UUID NOT NULL REFERENCES markets(id),
            buyer_account_id UUID NOT NULL REFERENCES accounts(id),
            seller_account_id UUID NOT NULL REFERENCES accounts(id),
            price BIGINT NOT NULL,
            qty BIGINT NOT NULL,
            fee BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create market_trades table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS solvers (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            successful_trades BIGINT NOT NULL DEFAULT 0,
            failed_trades BIGINT NOT NULL DEFAULT 0,
            total_volume BIGINT NOT NULL DEFAULT 0,
            reputation_score DOUBLE PRECISION NOT NULL DEFAULT 0.0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&pg_pool)
    .await
    .expect("Failed to create solvers table");

    // Market data service
    let market_data_repo = MarketDataRepository::new(pg_pool.clone());
    let market_data_service = Arc::new(MarketDataService::new(market_data_repo));

    // Solver reputation service
    let solver_repo = SolverRepository::new(pg_pool.clone());
    let solver_service = Arc::new(SolverReputationService::new(solver_repo));

    // Settlement engine
    let settlement_engine = Arc::new(SettlementEngine::new(pg_pool.clone()));

    // User service
    let user_repo = UserRepository::new(pg_pool);
    let user_service = Arc::new(UserService::new(user_repo, Arc::clone(&account_service)));

    // Shared storage
    let storage = Arc::new(Storage::new());

    // Each component gets its own EventBus (separate Redis connections)
    let intent_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for IntentService");
    let bid_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for BidService");
    let auction_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for AuctionEngine");
    let execution_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for ExecutionEngine");
    let ws_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for WsServer");

    // Risk engine
    let risk_engine = Arc::new(RiskEngine::new(
        Arc::clone(&balance_service),
        Arc::clone(&market_service),
    ));

    // Redis Streams event bus
    let stream_bus = Arc::new(
        StreamBus::new(&cfg.redis_url)
            .await
            .expect("Failed to connect Redis for StreamBus"),
    );

    for stream in crate::db::stream_bus::ALL_STREAMS {
        stream_bus
            .ensure_group(stream, "platform")
            .await
            .expect("Failed to create consumer group");
    }

    // Services
    let intent_service = Arc::new(Mutex::new(IntentService::new(
        Arc::clone(&storage),
        intent_bus,
        Arc::clone(&stream_bus),
        Arc::clone(&balance_service),
        Arc::clone(&risk_engine),
    )));
    let bid_service = Arc::new(Mutex::new(BidService::new(
        Arc::clone(&storage),
        bid_bus,
        Arc::clone(&stream_bus),
    )));

    // Engines
    let auction_engine = AuctionEngine::new(Arc::clone(&storage), auction_bus, cfg.auction_duration_secs);
    let execution_engine = ExecutionEngine::new(Arc::clone(&storage), execution_bus, cfg.execution_duration_secs);

    // Spawn stream consumer that forwards events to WebSocket feed
    let stream_consumer_bus = Arc::clone(&stream_bus);
    let stream_consumer_feed = {
        // ws_feed created below, need forward ref
        Arc::new(WsFeed::new())
    };
    let ws_feed = Arc::clone(&stream_consumer_feed);

    tokio::spawn(async move {
        if let Err(e) = stream_consumer_bus
            .subscribe(
                crate::db::stream_bus::ALL_STREAMS,
                "platform",
                "ws-forwarder",
                |event| {
                    let feed = Arc::clone(&stream_consumer_feed);
                    async move {
                        let msg = serde_json::json!({
                            "event": event.event_type,
                            "data": event.payload,
                            "event_id": event.event_id,
                            "timestamp": event.timestamp,
                        });
                        feed.broadcast_global(&msg.to_string());
                    }
                },
            )
            .await
        {
            tracing::error!(error = %e, "Stream consumer error");
        }
    });

    // WebSocket server (global Redis relay on /ws)
    let ws_server = WsServer::new();
    let ws_redis_client = ws_bus.client().clone();

    // Spawn background tasks
    tokio::spawn(async move {
        if let Err(e) = auction_engine.start().await {
            tracing::error!(error = %e, "AuctionEngine error");
        }
    });

    tokio::spawn(async move {
        if let Err(e) = execution_engine.start().await {
            tracing::error!(error = %e, "ExecutionEngine error");
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
        .merge(settlement::router(settlement_engine))
        .merge(markets::router(market_service))
        .merge(market_data::router(market_data_service))
        .merge(ws::router(ws_feed))
        .merge(solver_reputation::router(solver_service))
        .merge(metrics::router());

    // Start server
    let listener = tokio::net::TcpListener::bind(&cfg.server_addr)
        .await
        .expect("Failed to bind server address");
    tracing::info!(addr = %cfg.server_addr, "Server listening");
    axum::serve(listener, app).await.expect("Server error");
}
