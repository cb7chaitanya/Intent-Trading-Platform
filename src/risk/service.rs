use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::balances::model::Asset;
use crate::balances::service::BalanceService;
use crate::config;
use crate::markets::model::Market;
use crate::markets::service::MarketService;

#[derive(Debug, Clone)]
pub enum RiskRejection {
    InsufficientBalance { available: i64, required: u64 },
    BelowMinOrderSize { min: i64, got: u64 },
    PriceDeviationTooHigh { deviation_pct: f64, max_pct: f64 },
    MarketNotFound,
    MarketInactive,
    RateLimitExceeded { limit: u64 },
    DailyVolumeLimitExceeded { used: u64, limit: u64 },
    InvalidAsset(String),
}

impl std::fmt::Display for RiskRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskRejection::InsufficientBalance { available, required } => {
                write!(f, "Insufficient balance: have {available}, need {required}")
            }
            RiskRejection::BelowMinOrderSize { min, got } => {
                write!(f, "Order size {got} below minimum {min}")
            }
            RiskRejection::PriceDeviationTooHigh { deviation_pct, max_pct } => {
                write!(f, "Price deviation {deviation_pct:.1}% exceeds max {max_pct:.1}%")
            }
            RiskRejection::MarketNotFound => write!(f, "Market not found"),
            RiskRejection::MarketInactive => write!(f, "Market is not active"),
            RiskRejection::RateLimitExceeded { limit } => {
                write!(f, "Rate limit exceeded: max {limit} intents per minute")
            }
            RiskRejection::DailyVolumeLimitExceeded { used, limit } => {
                write!(f, "Daily volume {used} exceeds limit {limit}")
            }
            RiskRejection::InvalidAsset(a) => write!(f, "Invalid asset: {a}"),
        }
    }
}

/// Parameters for a single intent risk check.
pub struct IntentRiskParams {
    pub user_id: String,
    pub account_id: Uuid,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: u64,
    pub min_amount_out: u64,
}

/// Per-user rate + volume tracking.
struct UserActivity {
    /// Timestamps of recent intents (for rate limiting).
    recent_intents: Vec<std::time::Instant>,
    /// Cumulative volume today.
    daily_volume: u64,
    /// Day boundary for resetting daily volume.
    day_start: chrono::NaiveDate,
}

impl UserActivity {
    fn new() -> Self {
        Self {
            recent_intents: Vec::new(),
            daily_volume: 0,
            day_start: chrono::Utc::now().date_naive(),
        }
    }

    fn reset_if_new_day(&mut self) {
        let today = chrono::Utc::now().date_naive();
        if today != self.day_start {
            self.daily_volume = 0;
            self.day_start = today;
        }
    }

    fn prune_old_intents(&mut self) {
        let cutoff = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.recent_intents.retain(|t| *t > cutoff);
    }
}

pub struct RiskEngine {
    balance_service: Arc<BalanceService>,
    market_service: Arc<MarketService>,
    user_activity: Mutex<HashMap<String, UserActivity>>,
}

impl RiskEngine {
    pub fn new(
        balance_service: Arc<BalanceService>,
        market_service: Arc<MarketService>,
    ) -> Self {
        Self {
            balance_service,
            market_service,
            user_activity: Mutex::new(HashMap::new()),
        }
    }

    /// Run all risk checks against an intent. Returns Ok(Market) on success
    /// so the caller can use the resolved market without a second lookup.
    pub async fn validate_intent(
        &self,
        params: &IntentRiskParams,
    ) -> Result<Market, RiskRejection> {
        let asset_in = parse_asset(&params.token_in)?;
        let asset_out = parse_asset(&params.token_out)?;

        // 1. Resolve market
        let market = self.find_market(&asset_in, &asset_out).await?;

        // 2. Min order size
        if (params.amount_in as i64) < market.min_order_size {
            return Err(RiskRejection::BelowMinOrderSize {
                min: market.min_order_size,
                got: params.amount_in,
            });
        }

        // 3. Balance check
        self.check_balance(params.account_id, &asset_in, params.amount_in)
            .await?;

        // 4. Price deviation (implied price vs reasonable range)
        self.check_price_deviation(params.amount_in, params.min_amount_out, &market)?;

        // 5. Rate limit + daily volume
        self.check_user_limits(&params.user_id, params.amount_in)
            .await?;

        Ok(market)
    }

    /// Record that an intent was accepted (updates rate + volume counters).
    pub async fn record_accepted_intent(&self, user_id: &str, amount: u64) {
        let mut activity = self.user_activity.lock().await;
        let entry = activity
            .entry(user_id.to_string())
            .or_insert_with(UserActivity::new);
        entry.recent_intents.push(std::time::Instant::now());
        entry.reset_if_new_day();
        entry.daily_volume += amount;
    }

    async fn find_market(
        &self,
        asset_in: &Asset,
        asset_out: &Asset,
    ) -> Result<Market, RiskRejection> {
        let markets = self
            .market_service
            .list_markets()
            .await
            .map_err(|_| RiskRejection::MarketNotFound)?;

        markets
            .into_iter()
            .find(|m| {
                (&m.base_asset == asset_in && &m.quote_asset == asset_out)
                    || (&m.base_asset == asset_out && &m.quote_asset == asset_in)
            })
            .ok_or(RiskRejection::MarketNotFound)
    }

    async fn check_balance(
        &self,
        account_id: Uuid,
        asset: &Asset,
        required: u64,
    ) -> Result<(), RiskRejection> {
        let balances = self
            .balance_service
            .get_balances(account_id)
            .await
            .map_err(|_| RiskRejection::InsufficientBalance {
                available: 0,
                required,
            })?;

        let available = balances
            .iter()
            .find(|b| b.asset == *asset)
            .map(|b| b.available_balance)
            .unwrap_or(0);

        if available < required as i64 {
            return Err(RiskRejection::InsufficientBalance { available, required });
        }

        Ok(())
    }

    fn check_price_deviation(
        &self,
        amount_in: u64,
        min_amount_out: u64,
        market: &Market,
    ) -> Result<(), RiskRejection> {
        if min_amount_out == 0 || amount_in == 0 {
            return Ok(()); // Can't compute implied price
        }

        // Implied price = amount_in / min_amount_out
        // We check that min_amount_out is within a reasonable range relative
        // to tick_size as a rough proxy for market price.
        // If tick_size is very small relative to the ratio, the price is extreme.
        let implied_price = amount_in as f64 / min_amount_out as f64;

        // Use tick_size as a baseline unit. If the implied price is more than
        // config::get().max_price_deviation away from 1 tick unit, flag it.
        // This is a simplified check — in production you'd compare against
        // the last traded price or an oracle.
        if market.tick_size > 0 {
            let tick_price = market.tick_size as f64;
            // Only flag if the ratio is wildly off (e.g., someone asks for
            // 1 ETH and offers 1 USDC — a 99%+ deviation).
            if implied_price > 0.0 {
                let ratio = if implied_price > tick_price {
                    (implied_price - tick_price) / tick_price
                } else {
                    (tick_price - implied_price) / tick_price
                };

                if ratio > config::get().max_price_deviation * 100.0 {
                    return Err(RiskRejection::PriceDeviationTooHigh {
                        deviation_pct: ratio * 100.0,
                        max_pct: config::get().max_price_deviation * 100.0,
                    });
                }
            }
        }

        Ok(())
    }

    async fn check_user_limits(
        &self,
        user_id: &str,
        amount: u64,
    ) -> Result<(), RiskRejection> {
        let mut activity = self.user_activity.lock().await;
        let entry = activity
            .entry(user_id.to_string())
            .or_insert_with(UserActivity::new);

        // Rate limit
        entry.prune_old_intents();
        if entry.recent_intents.len() as u64 >= config::get().max_intents_per_minute {
            return Err(RiskRejection::RateLimitExceeded {
                limit: config::get().max_intents_per_minute,
            });
        }

        // Daily volume
        entry.reset_if_new_day();
        if entry.daily_volume + amount > config::get().daily_volume_limit {
            return Err(RiskRejection::DailyVolumeLimitExceeded {
                used: entry.daily_volume,
                limit: config::get().daily_volume_limit,
            });
        }

        Ok(())
    }
}

fn parse_asset(token: &str) -> Result<Asset, RiskRejection> {
    match token.to_uppercase().as_str() {
        "USDC" => Ok(Asset::USDC),
        "ETH" => Ok(Asset::ETH),
        "BTC" => Ok(Asset::BTC),
        "SOL" => Ok(Asset::SOL),
        other => Err(RiskRejection::InvalidAsset(other.to_string())),
    }
}
