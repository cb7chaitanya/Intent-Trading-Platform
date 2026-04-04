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

    // Run database migrations
    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pg_pool)
        .await
        .expect("Failed to run database migrations");
    tracing::info!("Migrations complete");

    // Account service
    let account_repo = AccountRepository::new(pg_pool.clone());
    let account_service = Arc::new(AccountService::new(account_repo));

    // Ledger service
    let ledger_repo = LedgerRepository::new(pg_pool.clone());
    let ledger_service = Arc::new(LedgerService::new(ledger_repo));

    // Balance service
    let balance_repo = BalanceRepository::new(pg_pool.clone());
    let balance_service = Arc::new(BalanceService::new(balance_repo, Arc::clone(&ledger_service)));

    // Market service
    let market_repo = MarketRepository::new(pg_pool.clone());
    let market_service = Arc::new(MarketService::new(market_repo));

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
