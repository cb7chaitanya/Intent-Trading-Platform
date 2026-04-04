mod accounts;
mod api;
mod api_keys;
mod auth;
mod balances;
mod cache;
mod config;
mod csrf;
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
mod oracle;
mod rbac;
mod risk;
mod services;
mod settlement;
mod shutdown;
mod solver_reputation;
mod twap;
mod users;
mod workers;
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

    // JWT key rotation service
    let key_rotation = Arc::new(auth::key_rotation::KeyRotationService::new(pg_pool.clone()));
    auth::jwt::init_key_service(Arc::clone(&key_rotation));

    // Redis cache service
    let cache_service = Arc::new(
        cache::CacheService::new(&cfg.redis_url)
            .await
            .expect("Failed to connect Redis for cache"),
    );

    // Account service
    let account_repo = AccountRepository::new(pg_pool.clone());
    let account_service = Arc::new(AccountService::new(account_repo));

    // Ledger service
    let ledger_repo = LedgerRepository::new(pg_pool.clone());
    let ledger_service = Arc::new(LedgerService::new(ledger_repo));

    // Balance service
    let balance_repo = BalanceRepository::new(pg_pool.clone());
    let balance_service = Arc::new(
        BalanceService::new(balance_repo, Arc::clone(&ledger_service))
            .with_cache(Arc::clone(&cache_service)),
    );

    // Market service
    let market_repo = MarketRepository::new(pg_pool.clone());
    let market_service = Arc::new(
        MarketService::new(market_repo).with_cache(Arc::clone(&cache_service)),
    );

    // Market data service
    let market_data_repo = MarketDataRepository::new(pg_pool.clone());
    let market_data_service = Arc::new(MarketDataService::new(market_data_repo));

    // Solver reputation service
    let solver_repo = SolverRepository::new(pg_pool.clone());
    let solver_service = Arc::new(
        SolverReputationService::new(solver_repo).with_cache(Arc::clone(&cache_service)),
    );

    // RBAC service
    let rbac_service = Arc::new(rbac::service::RbacService::new(pg_pool.clone()));

    // API key service
    let api_key_service = Arc::new(api_keys::service::ApiKeyService::new(pg_pool.clone()));

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
    let expiry_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for ExpiryWorker");
    let stop_bus = EventBus::new(&cfg.redis_url).await.expect("Failed to connect Redis for StopOrderMonitor");

    // Oracle service
    let oracle_service = Arc::new(oracle::service::OracleService::new(
        health_pool.clone(),
        Arc::clone(&market_service),
    ));

    // Risk engine
    let risk_engine = Arc::new(RiskEngine::new(
        Arc::clone(&balance_service),
        Arc::clone(&market_service),
        Arc::clone(&oracle_service),
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
        Arc::clone(&risk_engine),
    )));
    let bid_service = Arc::new(Mutex::new(BidService::new(
        Arc::clone(&storage),
        bid_bus,
        Arc::clone(&stream_bus),
    )));

    // Engines
    let auction_engine = AuctionEngine::new(Arc::clone(&storage), auction_bus, cfg.auction_duration_secs);
    let execution_engine = ExecutionEngine::new(
        Arc::clone(&storage),
        execution_bus,
        Arc::clone(&stream_bus),
        cfg.execution_duration_secs,
    );

    // Spawn stream consumer that forwards events to WebSocket feed
    let stream_consumer_bus = Arc::clone(&stream_bus);
    let stream_consumer_feed = Arc::new(WsFeed::new());
    let ws_feed = Arc::clone(&stream_consumer_feed);

    let mut bg_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Background task: oracle price feed
    let oracle_worker = Arc::clone(&oracle_service);
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        oracle_worker.run_price_feed(token).await;
    }));

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

    // Background task: settlement worker (auto-settles after execution)
    let settlement_worker_bus = Arc::clone(&stream_bus);
    let settlement_worker_engine = Arc::clone(&settlement_engine);
    let settlement_worker_pool = health_pool.clone();
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        settlement::worker::run(
            settlement_worker_bus,
            settlement_worker_engine,
            settlement_worker_pool,
            token,
        )
        .await;
    }));

    // Background task: settlement retry worker
    let retry_pool = settlement_engine.pool().clone();
    let retry_engine = Arc::clone(&settlement_engine);
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        settlement::retry::run_retry_worker(retry_pool, retry_engine, token).await;
    }));

    // Background task: intent expiry worker
    let expiry_pool = health_pool.clone();
    let expiry_event_bus = Arc::new(Mutex::new(expiry_bus));
    let expiry_stream_bus = Arc::clone(&stream_bus);
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        workers::intent_expiry::run(expiry_pool, expiry_event_bus, expiry_stream_bus, token).await;
    }));

    // Background task: JWT key rotation
    let key_rotation_worker = auth::key_rotation::KeyRotationService::new(health_pool.clone());
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        key_rotation_worker.run_rotation_worker(token).await;
    }));

    // TWAP service
    let twap_service = Arc::new(twap::service::TwapService::new(
        health_pool.clone(),
        Arc::clone(&intent_service),
    ));

    // Background task: TWAP scheduler
    let twap_scheduler_pool = health_pool.clone();
    let twap_scheduler_intent = Arc::clone(&intent_service);
    let twap_scheduler_svc = Arc::clone(&twap_service);
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        twap::scheduler::run(twap_scheduler_pool, twap_scheduler_intent, twap_scheduler_svc, token).await;
    }));

    // Background task: TWAP completion listener
    let twap_listener_bus = Arc::clone(&stream_bus);
    let twap_listener_svc = Arc::clone(&twap_service);
    let twap_listener_pool = health_pool.clone();
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        twap::listener::run(twap_listener_bus, twap_listener_svc, twap_listener_pool, token).await;
    }));

    // Background task: partition manager
    let partition_pool = health_pool.clone();
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        workers::partition_manager::run(partition_pool, token).await;
    }));

    // Background task: partition archival
    let archival_pool = health_pool.clone();
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        workers::partition_archival::run(archival_pool, token).await;
    }));

    // Background task: stop order monitor
    let stop_pool = health_pool.clone();
    let stop_oracle = Arc::clone(&oracle_service);
    let stop_event_bus = Arc::new(Mutex::new(stop_bus));
    let token = shutdown.token();
    bg_tasks.push(tokio::spawn(async move {
        workers::stop_order_monitor::run(stop_pool, stop_oracle, stop_event_bus, token).await;
    }));

    // Build combined router
    let app_state = AppState {
        intent_service,
        bid_service,
    };

    // Admin routes (JWT + admin role required)
    let admin_routes = rbac::router(Arc::clone(&rbac_service))
        .layer(axum::middleware::from_fn(rbac::middleware::require_role("admin")))
        .layer(axum::middleware::from_fn(auth::middleware::require_auth));

    // CSRF protection
    let csrf_state = Arc::new(csrf::middleware::CsrfState::new());

    // Protected routes (JWT + CSRF + idempotency)
    let protected = api::router(app_state)
        .merge(accounts::router(account_service))
        .merge(balances::router(balance_service))
        .merge(api_keys::router(api_key_service))
        .merge(ledger::router(ledger_service))
        .merge(settlement::router(settlement_engine))
        .merge(twap::router(twap_service))
        .layer(csrf::middleware::CsrfLayer::new(Arc::clone(&csrf_state)))
        .layer(idempotency::IdempotencyLayer::new(idempotency_pool))
        .layer(axum::middleware::from_fn(auth::middleware::require_auth));

    // Health check routes
    let health_state = health::HealthState {
        pg_pool: health_pool,
        redis_url: cfg.redis_url.clone(),
    };

    // Public routes (no JWT)
    let public = auth::public_router(Arc::clone(&user_service), Arc::clone(&rbac_service))
        .merge(health::router(health_state))
        .merge(users::router(user_service))
        .merge(markets::router(market_service))
        .merge(market_data::router(market_data_service))
        .merge(solver_reputation::router(solver_service))
        .merge(ws_server.router())
        .merge(ws::router(ws_feed))
        .merge(metrics::router())
        .merge(oracle::router(oracle_service))
        .merge(csrf::router(csrf_state));

    let app = admin_routes.merge(protected).merge(public);

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
