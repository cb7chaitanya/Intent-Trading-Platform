use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::balances::model::Asset;
use crate::balances::service::BalanceService;
use crate::config;
use crate::markets::model::Market;
use crate::markets::service::MarketService;
use crate::oracle::service::OracleService;

#[derive(Debug, Clone)]
pub enum RiskRejection {
    InsufficientBalance { available: i64, required: u64 },
    BelowMinOrderSize { min: i64, got: u64 },
    PriceDeviationTooHigh { deviation_pct: f64, max_pct: f64 },
    BidPriceDeviation { bid_price: f64, oracle_price: f64, deviation_pct: f64 },
    CrossMarketArbitrage { market_a_price: f64, market_b_price: f64, spread_pct: f64 },
    MarketNotFound,
    MarketInactive,
    RateLimitExceeded { limit: u64 },
    DailyVolumeLimitExceeded { used: u64, limit: u64 },
    InvalidAsset(String),
    MissingChainConfig(String),
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
            RiskRejection::BidPriceDeviation { bid_price, oracle_price, deviation_pct } => {
                write!(f, "Bid price {bid_price:.2} deviates {deviation_pct:.1}% from oracle {oracle_price:.2}")
            }
            RiskRejection::CrossMarketArbitrage { market_a_price, market_b_price, spread_pct } => {
                write!(f, "Cross-market arbitrage detected: prices {market_a_price:.2} vs {market_b_price:.2} ({spread_pct:.1}% spread)")
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
            RiskRejection::MissingChainConfig(msg) => write!(f, "Chain config: {msg}"),
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
    oracle: Arc<OracleService>,
    user_activity: Mutex<HashMap<String, UserActivity>>,
}

impl RiskEngine {
    pub fn new(
        balance_service: Arc<BalanceService>,
        market_service: Arc<MarketService>,
        oracle: Arc<OracleService>,
    ) -> Self {
        Self {
            balance_service,
            market_service,
            oracle,
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

        // 2. Validate chain config for on-chain settlement
        if market.chain != "ethereum" && market.chain != "solana" {
            return Err(RiskRejection::MissingChainConfig(format!(
                "Unknown chain: {}", market.chain
            )));
        }
        // Solana markets must have token mint addresses
        if market.is_solana() && market.base_token_mint.is_none() {
            return Err(RiskRejection::MissingChainConfig(
                "Solana market missing base_token_mint".into(),
            ));
        }

        // 3. Min order size
        if (params.amount_in as i64) < market.min_order_size {
            return Err(RiskRejection::BelowMinOrderSize {
                min: market.min_order_size,
                got: params.amount_in,
            });
        }

        // 4. Balance check
        self.check_balance(params.account_id, &asset_in, params.amount_in)
            .await?;

        // 5. Price deviation (implied price vs oracle price)
        self.check_price_deviation(params.amount_in, params.min_amount_out, &market).await?;

        // 6. Rate limit + daily volume
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

    async fn check_price_deviation(
        &self,
        amount_in: u64,
        min_amount_out: u64,
        market: &Market,
    ) -> Result<(), RiskRejection> {
        if min_amount_out == 0 || amount_in == 0 {
            return Ok(());
        }

        let implied_price = amount_in as f64 / min_amount_out as f64;

        // Try oracle price first, fall back to tick_size
        let reference_price = match self.oracle.get_price_value(&market.id).await {
            Some(oracle_price) if oracle_price > 0 => {
                tracing::debug!(
                    market_id = %market.id,
                    oracle_price = oracle_price,
                    "Using oracle price for deviation check"
                );
                oracle_price as f64
            }
            _ => {
                if market.tick_size > 0 {
                    market.tick_size as f64
                } else {
                    return Ok(()); // No reference price available
                }
            }
        };

        let max_deviation = config::get().max_price_deviation;

        let deviation = if implied_price > reference_price {
            (implied_price - reference_price) / reference_price
        } else {
            (reference_price - implied_price) / reference_price
        };

        if deviation > max_deviation {
            return Err(RiskRejection::PriceDeviationTooHigh {
                deviation_pct: deviation * 100.0,
                max_pct: max_deviation * 100.0,
            });
        }

        Ok(())
    }

    // ---------------------------------------------------------------
    // Bid validation (called by BidService before accepting a bid)
    // ---------------------------------------------------------------

    /// Validate a solver bid against oracle price and cross-market consistency.
    pub async fn validate_bid(
        &self,
        intent_token_in: &str,
        intent_token_out: &str,
        bid_amount_out: i64,
        bid_fee: i64,
        intent_amount_in: i64,
    ) -> Result<(), RiskRejection> {
        if intent_amount_in == 0 {
            return Ok(());
        }

        let asset_in = parse_asset(intent_token_in)?;
        let asset_out = parse_asset(intent_token_out)?;

        let market = self.find_market(&asset_in, &asset_out).await?;

        // Implied bid price = intent_amount_in / (bid_amount_out - bid_fee)
        let net_bid = bid_amount_out.saturating_sub(bid_fee);
        if net_bid <= 0 {
            return Ok(()); // degenerate bid, will lose auction anyway
        }
        let bid_price = intent_amount_in as f64 / net_bid as f64;

        // Check bid price vs oracle
        self.check_bid_vs_oracle(bid_price, &market).await?;

        // Check cross-market arbitrage
        self.check_cross_market(&asset_in, bid_price).await?;

        Ok(())
    }

    /// Reject bids whose implied price deviates too far from oracle.
    async fn check_bid_vs_oracle(
        &self,
        bid_price: f64,
        market: &Market,
    ) -> Result<(), RiskRejection> {
        let oracle_price = match self.oracle.get_price_value(&market.id).await {
            Some(p) if p > 0 => p as f64,
            _ => return Ok(()), // no oracle data — skip check
        };

        let max_dev = config::get().max_price_deviation;
        let deviation = (bid_price - oracle_price).abs() / oracle_price;

        if deviation > max_dev {
            tracing::warn!(
                market_id = %market.id,
                bid_price,
                oracle_price,
                deviation_pct = deviation * 100.0,
                "bid_price_deviation_rejected"
            );
            return Err(RiskRejection::BidPriceDeviation {
                bid_price,
                oracle_price,
                deviation_pct: deviation * 100.0,
            });
        }

        Ok(())
    }

    /// Compare oracle prices across markets with the same base asset.
    /// If two markets for the same base (e.g., ETH/USDC and ETH/BTC)
    /// show a price spread beyond threshold, flag potential arbitrage.
    async fn check_cross_market(
        &self,
        base_asset: &Asset,
        bid_price: f64,
    ) -> Result<(), RiskRejection> {
        let markets = match self.market_service.list_markets().await {
            Ok(m) => m,
            Err(_) => return Ok(()),
        };

        // Find all markets with the same base asset
        let related: Vec<&Market> = markets
            .iter()
            .filter(|m| &m.base_asset == base_asset)
            .collect();

        if related.len() < 2 {
            return Ok(()); // single market — no cross-market check needed
        }

        let max_spread = config::get().max_price_deviation * 2.0; // allow wider spread cross-market

        for market in &related {
            let oracle_price = match self.oracle.get_price_value(&market.id).await {
                Some(p) if p > 0 => p as f64,
                _ => continue,
            };

            let spread = (bid_price - oracle_price).abs() / oracle_price;

            if spread > max_spread {
                tracing::warn!(
                    base_asset = ?base_asset,
                    market_id = %market.id,
                    bid_price,
                    oracle_price,
                    spread_pct = spread * 100.0,
                    "cross_market_arbitrage_detected"
                );
                return Err(RiskRejection::CrossMarketArbitrage {
                    market_a_price: bid_price,
                    market_b_price: oracle_price,
                    spread_pct: spread * 100.0,
                });
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
