pub mod settings;

use once_cell::sync::OnceCell;

use self::settings::Settings;

static CONFIG: OnceCell<Settings> = OnceCell::new();

/// Initialize the global config. Call once at startup before anything else.
/// Loads from (in priority order, highest wins):
///   1. Defaults hardcoded in Settings
///   2. .env file
///   3. Environment variables
///   4. config.toml (if present)
pub fn init() -> &'static Settings {
    CONFIG.get_or_init(|| {
        // Load .env file if present (ignoring errors)
        let _ = dotenvy::dotenv();

        let builder = config::Config::builder()
            // Start with defaults (serde defaults handle this, but config crate
            // needs at least an empty base)
            .set_default("environment", "dev")
            .unwrap()
            // Layer on config.toml if it exists
            .add_source(
                config::File::with_name("config")
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            // Layer on environment variables (prefix ITX_, separator __)
            // e.g. ITX__DATABASE_URL, ITX__AUCTION_DURATION_SECS
            .add_source(
                config::Environment::with_prefix("ITX")
                    .separator("__")
                    .try_parsing(true),
            )
            // Also support bare env vars without prefix for common ones
            .add_source(
                config::Environment::default()
                    .try_parsing(true)
                    .prefix("")
                    .keep_prefix(false)
                    // Only pick up specific vars to avoid polluting config
                    .source(Some({
                        let mut map = std::collections::HashMap::new();
                        for key in [
                            "DATABASE_URL",
                            "REDIS_URL",
                            "SERVER_ADDR",
                            "GATEWAY_ADDR",
                            "UPSTREAM_URL",
                            "JWT_SECRET",
                            "LOG_LEVEL",
                            "ENVIRONMENT",
                        ] {
                            if let Ok(val) = std::env::var(key) {
                                map.insert(key.to_lowercase(), val);
                            }
                        }
                        map
                    })),
            );

        builder
            .build()
            .expect("Failed to build config")
            .try_deserialize::<Settings>()
            .expect("Failed to deserialize config")
    })
}

/// Get the global config. Panics if init() hasn't been called.
pub fn get() -> &'static Settings {
    CONFIG.get().expect("Config not initialized — call config::init() first")
}
