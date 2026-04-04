use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "default_database_url")]
    pub database_url: String,

    #[serde(default = "default_redis_url")]
    pub redis_url: String,

    #[serde(default = "default_server_addr")]
    pub server_addr: String,

    #[serde(default = "default_gateway_addr")]
    pub gateway_addr: String,

    #[serde(default = "default_upstream_url")]
    pub upstream_url: String,

    #[serde(default = "default_jwt_secret")]
    pub jwt_secret: String,

    #[serde(default = "default_auction_duration_secs")]
    pub auction_duration_secs: u64,

    #[serde(default = "default_execution_duration_secs")]
    pub execution_duration_secs: u64,

    #[serde(default = "default_fee_rate")]
    pub fee_rate: f64,

    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u64,

    #[serde(default = "default_rate_limit_window_secs")]
    pub rate_limit_window_secs: u64,

    #[serde(default = "default_daily_volume_limit")]
    pub daily_volume_limit: u64,

    #[serde(default = "default_max_price_deviation")]
    pub max_price_deviation: f64,

    #[serde(default = "default_max_intents_per_minute")]
    pub max_intents_per_minute: u64,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default = "default_environment")]
    pub environment: String,

    #[serde(default = "default_pg_max_connections")]
    pub pg_max_connections: u32,

    #[serde(default = "default_partition_retention_months")]
    pub partition_retention_months: i32,
}

fn default_database_url() -> String {
    "postgres://postgres:postgres@127.0.0.1:5432/intent_trading".to_string()
}
fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}
fn default_server_addr() -> String {
    "0.0.0.0:3000".to_string()
}
fn default_gateway_addr() -> String {
    "0.0.0.0:4000".to_string()
}
fn default_upstream_url() -> String {
    "http://127.0.0.1:3000".to_string()
}
fn default_jwt_secret() -> String {
    "change-me-in-production".to_string()
}
fn default_auction_duration_secs() -> u64 {
    10
}
fn default_execution_duration_secs() -> u64 {
    3
}
fn default_fee_rate() -> f64 {
    0.001
}
fn default_rate_limit_per_minute() -> u64 {
    120
}
fn default_rate_limit_window_secs() -> u64 {
    60
}
fn default_daily_volume_limit() -> u64 {
    10_000_000
}
fn default_max_price_deviation() -> f64 {
    0.20
}
fn default_max_intents_per_minute() -> u64 {
    30
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_environment() -> String {
    "dev".to_string()
}
fn default_pg_max_connections() -> u32 {
    5
}
fn default_partition_retention_months() -> i32 {
    6
}
