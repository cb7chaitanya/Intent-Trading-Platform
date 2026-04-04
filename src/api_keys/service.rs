use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Prefix length stored in DB for fast lookup before hash comparison.
const KEY_PREFIX_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub user_id: Uuid,
    #[serde(skip)]
    pub key_hash: String,
    pub key_prefix: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

#[derive(Debug, Serialize)]
pub struct CreateKeyResponse {
    pub id: Uuid,
    pub key: String,
    pub name: String,
}

#[derive(Debug)]
pub enum ApiKeyError {
    NotFound,
    Revoked,
    InvalidKey,
    DbError(sqlx::Error),
    HashError(String),
}

impl std::fmt::Display for ApiKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiKeyError::NotFound => write!(f, "API key not found"),
            ApiKeyError::Revoked => write!(f, "API key has been revoked"),
            ApiKeyError::InvalidKey => write!(f, "Invalid API key"),
            ApiKeyError::DbError(e) => write!(f, "Database error: {e}"),
            ApiKeyError::HashError(e) => write!(f, "Hash error: {e}"),
        }
    }
}

impl From<sqlx::Error> for ApiKeyError {
    fn from(e: sqlx::Error) -> Self {
        ApiKeyError::DbError(e)
    }
}

pub struct ApiKeyService {
    pool: PgPool,
}

impl ApiKeyService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new API key for a user. Returns the raw key (only shown once).
    pub async fn create_key(
        &self,
        user_id: Uuid,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<CreateKeyResponse, ApiKeyError> {
        let raw_key = generate_key();
        let prefix = &raw_key[..KEY_PREFIX_LEN];
        let hash = hash_key(&raw_key)?;

        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, key_hash, key_prefix, name, permissions, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())",
        )
        .bind(id)
        .bind(user_id)
        .bind(&hash)
        .bind(prefix)
        .bind(name)
        .bind(&permissions)
        .execute(&self.pool)
        .await?;

        tracing::info!(user_id = %user_id, key_id = %id, name = name, "api_key_created");

        Ok(CreateKeyResponse {
            id,
            key: raw_key,
            name: name.to_string(),
        })
    }

    /// Validate an API key. Returns the key record if valid.
    pub async fn validate_key(&self, raw_key: &str) -> Result<ApiKey, ApiKeyError> {
        if raw_key.len() < KEY_PREFIX_LEN {
            return Err(ApiKeyError::InvalidKey);
        }

        let prefix = &raw_key[..KEY_PREFIX_LEN];

        // Find candidates by prefix (fast index lookup)
        let candidates = sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE key_prefix = $1 AND revoked = FALSE",
        )
        .bind(prefix)
        .fetch_all(&self.pool)
        .await?;

        let hash = hash_key(raw_key)?;

        // Compare hash (constant-time comparison via bcrypt verify would be ideal,
        // but SHA256 comparison is fine since the key is high-entropy)
        let key = candidates
            .into_iter()
            .find(|k| k.key_hash == hash)
            .ok_or(ApiKeyError::NotFound)?;

        if key.revoked {
            return Err(ApiKeyError::Revoked);
        }

        // Update last_used_at (best-effort)
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(key.id)
            .execute(&self.pool)
            .await;

        Ok(key)
    }

    /// List all API keys for a user (without hashes).
    pub async fn list_keys(&self, user_id: Uuid) -> Result<Vec<ApiKey>, ApiKeyError> {
        let keys = sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(keys)
    }

    /// Revoke an API key.
    pub async fn revoke_key(&self, key_id: Uuid, user_id: Uuid) -> Result<(), ApiKeyError> {
        let result = sqlx::query(
            "UPDATE api_keys SET revoked = TRUE WHERE id = $1 AND user_id = $2",
        )
        .bind(key_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(ApiKeyError::NotFound);
        }

        tracing::info!(key_id = %key_id, user_id = %user_id, "api_key_revoked");
        Ok(())
    }
}

/// Generate a random API key: itx_<32 hex chars>
fn generate_key() -> String {
    let random_bytes: [u8; 16] = rand::random();
    format!("itx_{}", hex::encode(random_bytes))
}

/// SHA256 hash of the key for storage.
fn hash_key(key: &str) -> Result<String, ApiKeyError> {
    use sha2::Digest;
    let result = sha2::Sha256::digest(key.as_bytes());
    Ok(hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_correct_format() {
        let key = generate_key();
        assert!(key.starts_with("itx_"));
        assert_eq!(key.len(), 4 + 32); // "itx_" + 32 hex chars
    }

    #[test]
    fn hash_is_deterministic() {
        let key = "itx_abcdef1234567890abcdef12345678";
        let h1 = hash_key(key).unwrap();
        let h2 = hash_key(key).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_keys_different_hashes() {
        let h1 = hash_key("itx_aaaa").unwrap();
        let h2 = hash_key("itx_bbbb").unwrap();
        assert_ne!(h1, h2);
    }
}
