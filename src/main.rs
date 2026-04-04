mod accounts;
mod api;
mod auth;
mod balances;
mod config;
mod db;
mod ledger;
mod engine;
mod fees;
mod health;
mod idempotency;
mod market_data;
mod markets;
mod metrics;
mod models;
mod risk;
mod services;
mod settlement;
mod shutdown;
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
use crate::shutdown::Shutdown;
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

    // Shutdown coordinator
    let shutdown = Shutdown::new();

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
    let user_repo = UserRepository::new(pg_pool.clone());
    let user_service = Arc::new(UserService::new(user_repo, Arc::clone(&account_service)));

    // Postgres-backed storage for intents, bids, fills, executions
    let health_pool = pg_pool.clone();
    let idempotency_pool = pg_pool.clone();
    let storage = Arc::new(Storage::new(pg_pool));

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
    let stream_consumer_feed = Arc::new(WsFeed::new());
    let ws_feed = Arc::clone(&stream_consumer_feed);

    let mut bg_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Background task: stream consumer → WS forwarder
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        tokio::select! {
            result = stream_consumer_bus.subscribe(
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
            ) => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "Stream consumer error");
                }
            }
            _ = token.cancelled() => {
                tracing::info!("Stream consumer shutting down");
            }
        }
    }));

    // Background task: auction engine
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        tokio::select! {
            result = auction_engine.start() => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "AuctionEngine error");
                }
            }
            _ = token.cancelled() => {
                tracing::info!("AuctionEngine shutting down");
            }
        }
    }));

    // Background task: execution engine
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        tokio::select! {
            result = execution_engine.start() => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "ExecutionEngine error");
                }
            }
            _ = token.cancelled() => {
                tracing::info!("ExecutionEngine shutting down");
            }
        }
    }));

    // Background task: WS Redis relay
    let ws_server = WsServer::new();
    let ws_redis_client = ws_bus.client().clone();
    let ws_listener = ws_server.clone();
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        tokio::select! {
            _ = ws_listener.start_redis_listener(&ws_redis_client) => {}
            _ = token.cancelled() => {
                tracing::info!("WS Redis listener shutting down");
            }
        }
    }));

    // Background task: settlement retry worker
    let retry_pool = settlement_engine.pool().clone();
    let retry_engine = Arc::clone(&settlement_engine);
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        settlement::retry::run_retry_worker(retry_pool, retry_engine, token).await;
    }));

    // Build combined router
    let app_state = AppState {
        intent_service,
        bid_service,
    };

    // Protected routes (JWT required, idempotency-checked)
    let protected = api::router(app_state)
        .merge(accounts::router(account_service))
        .merge(balances::router(balance_service))
        .merge(ledger::router(ledger_service))
        .merge(settlement::router(settlement_engine))
        .layer(idempotency::IdempotencyLayer::new(idempotency_pool))
        .layer(axum::middleware::from_fn(auth::middleware::require_auth));

    // Health check routes
    let health_state = health::HealthState {
        pg_pool: health_pool,
        redis_url: cfg.redis_url.clone(),
    };

    // Public routes (no JWT)
    let public = auth::public_router(Arc::clone(&user_service))
        .merge(health::router(health_state))
        .merge(users::router(user_service))
        .merge(markets::router(market_service))
        .merge(market_data::router(market_data_service))
        .merge(solver_reputation::router(solver_service))
        .merge(ws_server.router())
        .merge(ws::router(ws_feed))
        .merge(metrics::router());

    let app = protected.merge(public);

    // Start server with graceful shutdown
    let listener = tokio::net::TcpListener::bind(&cfg.server_addr)
        .await
        .expect("Failed to bind server address");
    tracing::info!(addr = %cfg.server_addr, "Server listening");

    let shutdown_for_signal = shutdown.clone();
    tokio::spawn(async move {
        shutdown_for_signal.listen_for_signals().await;
    });

    let shutdown_token = shutdown.token();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_token.cancelled().await;
            tracing::info!("HTTP server stopping — draining in-flight requests");
        })
        .await
        .expect("Server error");

    // Server has stopped accepting new connections.
    // Wait for background tasks to finish gracefully.
    tracing::info!("Server stopped, cleaning up background tasks...");
    shutdown.trigger(); // ensure all tasks see cancellation
    shutdown.wait_for_completion(bg_tasks).await;

    tracing::info!("Shutdown complete");
}
