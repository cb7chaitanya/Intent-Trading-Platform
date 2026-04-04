use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config;

use super::key_rotation::KeyRotationService;

const TOKEN_EXPIRY_SECS: i64 = 86400; // 24 hours

/// Global key rotation service, set once at startup.
static KEY_SERVICE: OnceCell<Arc<KeyRotationService>> = OnceCell::new();

/// Cached active key to avoid DB lookups on every request.
static ACTIVE_KEY_CACHE: OnceCell<RwLock<Option<CachedKey>>> = OnceCell::new();

struct CachedKey {
    secret: String,
    fetched_at: std::time::Instant,
}

const KEY_CACHE_TTL_SECS: u64 = 60;

/// Initialize the JWT module with a key rotation service.
/// Call once at startup after DB is ready.
pub fn init_key_service(svc: Arc<KeyRotationService>) {
    let _ = KEY_SERVICE.set(svc);
    let _ = ACTIVE_KEY_CACHE.set(RwLock::new(None));
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub permissions: Vec<String>,
    pub exp: i64,
    pub iat: i64,
    #[serde(default)]
    pub kid: Option<String>,
}

#[derive(Debug)]
pub enum JwtError {
    EncodingFailed(String),
    InvalidToken(String),
    Expired,
    NoSigningKey,
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwtError::EncodingFailed(e) => write!(f, "Token encoding failed: {e}"),
            JwtError::InvalidToken(e) => write!(f, "Invalid token: {e}"),
            JwtError::Expired => write!(f, "Token expired"),
            JwtError::NoSigningKey => write!(f, "No signing key available"),
        }
    }
}

/// Get the active signing key, with in-memory caching.
async fn get_signing_secret() -> Result<String, JwtError> {
    // Try key rotation service first
    if let Some(svc) = KEY_SERVICE.get() {
        let cache_lock = ACTIVE_KEY_CACHE.get().unwrap();
        {
            let cache = cache_lock.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.fetched_at.elapsed().as_secs() < KEY_CACHE_TTL_SECS {
                    return Ok(cached.secret.clone());
                }
            }
        }

        // Cache miss or expired — fetch from DB
        let key = svc.get_active_key().await.map_err(|e| JwtError::EncodingFailed(e.to_string()))?;

        let mut cache = cache_lock.write().await;
        *cache = Some(CachedKey {
            secret: key.key_secret.clone(),
            fetched_at: std::time::Instant::now(),
        });

        return Ok(key.key_secret);
    }

    // Fallback to config secret (for tests / gateway binary without DB)
    Ok(config::get().jwt_secret.clone())
}

/// Get all valid secrets for token validation (active + grace period).
async fn get_validation_secrets() -> Vec<String> {
    if let Some(svc) = KEY_SERVICE.get() {
        if let Ok(keys) = svc.get_validation_keys().await {
            return keys.into_iter().map(|k| k.key_secret).collect();
        }
    }

    // Fallback
    vec![config::get().jwt_secret.clone()]
}

pub async fn create_token(
    user_id: Uuid,
    email: &str,
    permissions: Vec<String>,
) -> Result<String, JwtError> {
    let secret = get_signing_secret().await?;
    let now = chrono::Utc::now().timestamp();

    let claims = Claims {
        sub: user_id,
        email: email.to_string(),
        permissions,
        exp: now + TOKEN_EXPIRY_SECS,
        iat: now,
        kid: None,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| JwtError::EncodingFailed(e.to_string()))
}

pub async fn validate_token(token: &str) -> Result<Claims, JwtError> {
    let secrets = get_validation_secrets().await;

    // Try each valid key (active first, then grace period keys)
    for secret in &secrets {
        match decode::<Claims>(
            token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        ) {
            Ok(data) => return Ok(data.claims),
            Err(e) => match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    return Err(JwtError::Expired);
                }
                jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                    continue; // try next key
                }
                _ => continue,
            },
        }
    }

    Err(JwtError::InvalidToken("No valid signing key found".into()))
}

/// Synchronous fallback for contexts without async (gateway lib.rs re-export).
/// Uses config secret only — no DB key rotation.
pub fn validate_token_sync(token: &str) -> Result<Claims, JwtError> {
    let secret = &config::get().jwt_secret;

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
        _ => JwtError::InvalidToken(e.to_string()),
    })?;

    Ok(token_data.claims)
}
