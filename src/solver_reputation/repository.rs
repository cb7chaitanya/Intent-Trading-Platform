use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::Solver;

pub struct SolverRepository {
    pool: PgPool,
}

impl SolverRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Register a brand-new solver with an API key.
    pub async fn register(
        &self,
        id: Uuid,
        name: &str,
        email: Option<&str>,
        webhook_url: Option<&str>,
        api_key: &str,
    ) -> Result<Solver, sqlx::Error> {
        let now = Utc::now();
        sqlx::query_as::<_, Solver>(
            "INSERT INTO solvers
                (id, name, email, webhook_url, api_key, active,
                 successful_trades, failed_trades, total_volume,
                 total_fills, failed_fills, reputation_score, created_at)
             VALUES ($1,$2,$3,$4,$5,TRUE,0,0,0,0,0,0.0,$6)
             RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(email)
        .bind(webhook_url)
        .bind(api_key)
        .bind(now)
        .fetch_one(&self.pool)
        .await
    }

    /// Look up by name to enforce uniqueness before insert.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
    }

    /// Look up by API key (used for solver auth).
    pub async fn find_by_api_key(&self, api_key: &str) -> Result<Option<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE api_key = $1 AND active = TRUE")
            .bind(api_key)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn find_or_create(&self, id: Uuid, name: &str) -> Result<Solver, sqlx::Error> {
        let existing = sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(solver) = existing {
            return Ok(solver);
        }

        let now = Utc::now();
        sqlx::query_as::<_, Solver>(
            "INSERT INTO solvers
                (id, name, active, successful_trades, failed_trades, total_volume,
                 total_fills, failed_fills, reputation_score, created_at)
             VALUES ($1,$2,TRUE,0,0,0,0,0,0.0,$3)
             ON CONFLICT (id) DO NOTHING
             RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map(|opt| {
            opt.unwrap_or(Solver {
                id,
                name: name.to_string(),
                email: None,
                api_key: None,
                webhook_url: None,
                active: true,
                successful_trades: 0,
                failed_trades: 0,
                total_volume: 0,
                total_fills: 0,
                failed_fills: 0,
                reputation_score: 0.0,
                created_at: now,
            })
        })
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn update(&self, solver: &Solver) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE solvers
             SET successful_trades = $1, failed_trades = $2, total_volume = $3,
                 reputation_score = $4, total_fills = $5, failed_fills = $6
             WHERE id = $7",
        )
        .bind(solver.successful_trades)
        .bind(solver.failed_trades)
        .bind(solver.total_volume)
        .bind(solver.reputation_score)
        .bind(solver.total_fills)
        .bind(solver.failed_fills)
        .bind(solver.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_profile(
        &self,
        id: Uuid,
        name: Option<&str>,
        email: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<Option<Solver>, sqlx::Error> {
        // Only update fields that are Some
        sqlx::query_as::<_, Solver>(
            "UPDATE solvers
             SET name       = COALESCE($2, name),
                 email      = COALESCE($3, email),
                 webhook_url = COALESCE($4, webhook_url)
             WHERE id = $1
             RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(email)
        .bind(webhook_url)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn deactivate(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("UPDATE solvers SET active = FALSE WHERE id = $1 AND active = TRUE")
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn find_top(&self, limit: i64) -> Result<Vec<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>(
            "SELECT * FROM solvers WHERE active = TRUE ORDER BY reputation_score DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn find_all(&self) -> Result<Vec<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>("SELECT * FROM solvers ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
    }
}
