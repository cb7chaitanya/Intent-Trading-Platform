use std::sync::Arc;

use uuid::Uuid;

use crate::cache::service::{CacheService, CacheTtl};

use super::model::Solver;
use super::repository::SolverRepository;

#[derive(Debug)]
pub enum SolverError {
    NotFound,
    DbError(sqlx::Error),
}

impl std::fmt::Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolverError::NotFound => write!(f, "Solver not found"),
            SolverError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for SolverError {
    fn from(e: sqlx::Error) -> Self {
        SolverError::DbError(e)
    }
}

pub struct SolverReputationService {
    repo: SolverRepository,
    cache: Option<Arc<CacheService>>,
}

impl SolverReputationService {
    pub fn new(repo: SolverRepository) -> Self {
        Self { repo, cache: None }
    }

    pub fn with_cache(mut self, cache: Arc<CacheService>) -> Self {
        self.cache = Some(cache);
        self
    }

    fn invalidate_leaderboard(&self) {
        if let Some(cache) = &self.cache {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache.invalidate_pattern("leaderboard").await;
            });
        }
    }

    pub async fn record_successful_trade(
        &self,
        solver_id: Uuid,
        solver_name: &str,
        volume: i64,
    ) -> Result<Solver, SolverError> {
        let mut solver = self.repo.find_or_create(solver_id, solver_name).await?;
        solver.successful_trades += 1;
        solver.total_volume += volume;
        solver.reputation_score = calculate_reputation(&solver);
        self.repo.update(&solver).await?;
        self.invalidate_leaderboard();
        Ok(solver)
    }

    pub async fn record_failed_trade(
        &self,
        solver_id: Uuid,
        solver_name: &str,
    ) -> Result<Solver, SolverError> {
        let mut solver = self.repo.find_or_create(solver_id, solver_name).await?;
        solver.failed_trades += 1;
        solver.reputation_score = calculate_reputation(&solver);
        self.repo.update(&solver).await?;
        self.invalidate_leaderboard();
        Ok(solver)
    }

    pub async fn get_solver(&self, solver_id: Uuid) -> Result<Solver, SolverError> {
        self.repo
            .find_by_id(solver_id)
            .await?
            .ok_or(SolverError::NotFound)
    }

    pub async fn get_top_solvers(&self, limit: i64) -> Result<Vec<Solver>, SolverError> {
        let limit = limit.clamp(1, 100);
        let key = format!("top_{limit}");

        if let Some(cache) = &self.cache {
            if let Some(solvers) = cache.get::<Vec<Solver>>("leaderboard", &key).await {
                return Ok(solvers);
            }
        }

        let solvers = self.repo.find_top(limit).await?;

        if let Some(cache) = &self.cache {
            cache.set("leaderboard", &key, &solvers, CacheTtl::LEADERBOARD).await;
        }

        Ok(solvers)
    }
}

fn calculate_reputation(solver: &Solver) -> f64 {
    let total = solver.successful_trades + solver.failed_trades;
    if total == 0 { return 0.0; }
    let success_rate = solver.successful_trades as f64 / total as f64;
    let activity = (1.0 + total as f64).log2();
    let volume_bonus = 1.0 + (1.0 + solver.total_volume as f64).log10() / 20.0;
    let score = success_rate * activity * volume_bonus;
    (score * 10000.0).round() / 10000.0
}
