use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ROTATION_INTERVAL_SECS: u64 = 3600; // check every hour
const KEY_MAX_AGE_DAYS: i64 = 7;
const KEY_CLEANUP_DAYS: i64 = 30;
const KEY_LENGTH: usize = 64;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct JwtKey {
    pub id: Uuid,
    pub key_secret: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

pub struct KeyRotationService {
    pool: PgPool,
}

impl KeyRotationService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get the active signing key. Creates one if none exists.
    pub async fn get_active_key(&self) -> Result<JwtKey, sqlx::Error> {
        let key = sqlx::query_as::<_, JwtKey>(
            "SELECT * FROM jwt_keys WHERE active = TRUE ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        match key {
            Some(k) => Ok(k),
            None => self.create_and_activate_key().await,
        }
    }

    /// Get all keys valid for token verification (active + recent inactive within grace period).
    pub async fn get_validation_keys(&self) -> Result<Vec<JwtKey>, sqlx::Error> {
        let grace_cutoff = Utc::now() - Duration::days(KEY_MAX_AGE_DAYS);

        sqlx::query_as::<_, JwtKey>(
            "SELECT * FROM jwt_keys
             WHERE active = TRUE OR created_at >= $1
             ORDER BY active DESC, created_at DESC",
        )
        .bind(grace_cutoff)
        .fetch_all(&self.pool)
        .await
    }

    /// Rotate the key if the active one is older than KEY_MAX_AGE_DAYS.
    pub async fn rotate_if_needed(&self) -> Result<bool, sqlx::Error> {
        let active = self.get_active_key().await?;
        let age = Utc::now() - active.created_at;

        if age.num_days() < KEY_MAX_AGE_DAYS {
            return Ok(false);
        }

        tracing::info!(
            key_id = %active.id,
            age_days = age.num_days(),
            "Rotating JWT signing key"
        );

        // Deactivate current key
        sqlx::query("UPDATE jwt_keys SET active = FALSE WHERE id = $1")
            .bind(active.id)
            .execute(&self.pool)
            .await?;

        // Create new active key
        let new_key = self.create_and_activate_key().await?;

        tracing::info!(
            old_key = %active.id,
            new_key = %new_key.id,
            "jwt_key_rotated"
        );

        Ok(true)
    }

    /// Delete keys older than KEY_CLEANUP_DAYS that are inactive.
    pub async fn cleanup_old_keys(&self) -> Result<u64, sqlx::Error> {
        let cutoff = Utc::now() - Duration::days(KEY_CLEANUP_DAYS);

        let result = sqlx::query(
            "DELETE FROM jwt_keys WHERE active = FALSE AND created_at < $1",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected();
        if deleted > 0 {
            tracing::info!(deleted, "Old JWT keys cleaned up");
        }

        Ok(deleted)
    }

    async fn create_and_activate_key(&self) -> Result<JwtKey, sqlx::Error> {
        let secret = generate_secret();
        let key = JwtKey {
            id: Uuid::new_v4(),
            key_secret: secret,
            active: true,
            created_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO jwt_keys (id, key_secret, active, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(key.id)
        .bind(&key.key_secret)
        .bind(key.active)
        .bind(key.created_at)
        .execute(&self.pool)
        .await?;

        tracing::info!(key_id = %key.id, "New JWT signing key created");
        Ok(key)
    }

    /// Background worker that rotates keys and cleans up old ones.
    pub async fn run_rotation_worker(self, cancel: CancellationToken) {
        tracing::info!("JWT key rotation worker started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(ROTATION_INTERVAL_SECS)) => {}
                _ = cancel.cancelled() => {
                    tracing::info!("JWT key rotation worker shutting down");
                    return;
                }
            }

            if let Err(e) = self.rotate_if_needed().await {
                tracing::error!(error = %e, "Key rotation check failed");
            }

            if let Err(e) = self.cleanup_old_keys().await {
                tracing::error!(error = %e, "Key cleanup failed");
            }
        }
    }
}

fn generate_secret() -> String {
    let bytes: Vec<u8> = (0..KEY_LENGTH).map(|_| rand::random::<u8>()).collect();
    hex::encode(bytes)
}
