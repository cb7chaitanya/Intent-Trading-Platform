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

    pub async fn find_or_create(&self, id: Uuid, name: &str) -> Result<Solver, sqlx::Error> {
        let existing = sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(solver) = existing {
            return Ok(solver);
        }

        let solver = Solver {
            id,
            name: name.to_string(),
            successful_trades: 0,
            failed_trades: 0,
            total_volume: 0,
            reputation_score: 0.0,
            created_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO solvers (id, name, successful_trades, failed_trades, total_volume, reputation_score, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(solver.id)
        .bind(&solver.name)
        .bind(solver.successful_trades)
        .bind(solver.failed_trades)
        .bind(solver.total_volume)
        .bind(solver.reputation_score)
        .bind(solver.created_at)
        .execute(&self.pool)
        .await?;

        Ok(solver)
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>("SELECT * FROM solvers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn update(&self, solver: &Solver) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE solvers SET successful_trades = $1, failed_trades = $2, total_volume = $3, reputation_score = $4
             WHERE id = $5",
        )
        .bind(solver.successful_trades)
        .bind(solver.failed_trades)
        .bind(solver.total_volume)
        .bind(solver.reputation_score)
        .bind(solver.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_top(&self, limit: i64) -> Result<Vec<Solver>, sqlx::Error> {
        sqlx::query_as::<_, Solver>(
            "SELECT * FROM solvers ORDER BY reputation_score DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }
}
