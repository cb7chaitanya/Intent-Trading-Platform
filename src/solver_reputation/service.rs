use std::sync::Arc;

use uuid::Uuid;

use crate::cache::service::{CacheService, CacheTtl};

use super::model::{Solver, SolverDashboard, SolverPublic};
use super::repository::SolverRepository;

#[derive(Debug)]
pub enum SolverError {
    NotFound,
    NameTaken,
    DbError(sqlx::Error),
}

impl std::fmt::Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolverError::NotFound => write!(f, "Solver not found"),
            SolverError::NameTaken => write!(f, "Solver name already taken"),
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

    // ── Registration ──────────────────────────────────────

    pub async fn register(
        &self,
        name: &str,
        email: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<(Solver, String), SolverError> {
        // Check name uniqueness
        if self.repo.find_by_name(name).await?.is_some() {
            return Err(SolverError::NameTaken);
        }

        let id = Uuid::new_v4();
        let api_key = generate_api_key();

        let solver = self.repo.register(id, name, email, webhook_url, &api_key).await?;
        Ok((solver, api_key))
    }

    /// Authenticate a solver by API key.
    pub async fn authenticate(&self, api_key: &str) -> Result<Solver, SolverError> {
        self.repo
            .find_by_api_key(api_key)
            .await?
            .ok_or(SolverError::NotFound)
    }

    // ── Profile ───────────────────────────────────────────

    pub async fn update_profile(
        &self,
        solver_id: Uuid,
        name: Option<&str>,
        email: Option<&str>,
        webhook_url: Option<&str>,
    ) -> Result<Solver, SolverError> {
        // If renaming, check name uniqueness
        if let Some(new_name) = name {
            if let Some(existing) = self.repo.find_by_name(new_name).await? {
                if existing.id != solver_id {
                    return Err(SolverError::NameTaken);
                }
            }
        }
        self.repo
            .update_profile(solver_id, name, email, webhook_url)
            .await?
            .ok_or(SolverError::NotFound)
    }

    pub async fn deactivate(&self, solver_id: Uuid) -> Result<(), SolverError> {
        if !self.repo.deactivate(solver_id).await? {
            return Err(SolverError::NotFound);
        }
        self.invalidate_leaderboard();
        Ok(())
    }

    // ── Dashboard ─────────────────────────────────────────

    pub async fn get_dashboard(&self, solver_id: Uuid) -> Result<SolverDashboard, SolverError> {
        let solver = self
            .repo
            .find_by_id(solver_id)
            .await?
            .ok_or(SolverError::NotFound)?;

        let total_trades = solver.successful_trades + solver.failed_trades;
        let win_rate = if total_trades > 0 {
            solver.successful_trades as f64 / total_trades as f64 * 100.0
        } else {
            0.0
        };

        let total_fill_attempts = solver.total_fills + solver.failed_fills;
        let fill_success_rate = if total_fill_attempts > 0 {
            solver.total_fills as f64 / total_fill_attempts as f64 * 100.0
        } else {
            0.0
        };

        let avg_volume_per_trade = if solver.successful_trades > 0 {
            solver.total_volume as f64 / solver.successful_trades as f64
        } else {
            0.0
        };

        Ok(SolverDashboard {
            solver: SolverPublic::from(solver),
            win_rate,
            fill_success_rate,
            avg_volume_per_trade,
        })
    }

    // ── Trade recording ───────────────────────────────────

    pub async fn record_successful_trade(
        &self,
        solver_id: Uuid,
        solver_name: &str,
        volume: i64,
    ) -> Result<Solver, SolverError> {
        let mut solver = self.repo.find_or_create(solver_id, solver_name).await?;
        solver.successful_trades += 1;
        solver.total_volume += volume;
        solver.total_fills += 1;
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
        solver.failed_fills += 1;
        solver.reputation_score = calculate_reputation(&solver);
        self.repo.update(&solver).await?;
        self.invalidate_leaderboard();
        Ok(solver)
    }

    /// Record a successful fill without changing trade win/loss stats.
    pub async fn record_fill(
        &self,
        solver_id: Uuid,
        solver_name: &str,
        volume: i64,
    ) -> Result<Solver, SolverError> {
        let mut solver = self.repo.find_or_create(solver_id, solver_name).await?;
        solver.total_fills += 1;
        solver.total_volume += volume;
        solver.reputation_score = calculate_reputation(&solver);
        self.repo.update(&solver).await?;
        self.invalidate_leaderboard();
        Ok(solver)
    }

    /// Penalty for a failed settlement — hurts reputation.
    pub async fn penalize_failed_settlement(
        &self,
        solver_id: Uuid,
        solver_name: &str,
    ) -> Result<Solver, SolverError> {
        let mut solver = self.repo.find_or_create(solver_id, solver_name).await?;
        solver.failed_fills += 1;
        solver.failed_trades += 1;
        solver.reputation_score = calculate_reputation(&solver);
        self.repo.update(&solver).await?;
        self.invalidate_leaderboard();
        Ok(solver)
    }

    // ── Read ──────────────────────────────────────────────

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
            cache
                .set("leaderboard", &key, &solvers, CacheTtl::LEADERBOARD)
                .await;
        }

        Ok(solvers)
    }
}

// ── Reputation formula ───────────────────────────────────

pub fn calculate_reputation(solver: &Solver) -> f64 {
    let total = solver.successful_trades + solver.failed_trades;
    if total == 0 {
        return 0.0;
    }
    let success_rate = solver.successful_trades as f64 / total as f64;

    // Reliability factor: penalise failed fills
    let fill_attempts = solver.total_fills + solver.failed_fills;
    let reliability = if fill_attempts > 0 {
        solver.total_fills as f64 / fill_attempts as f64
    } else {
        1.0
    };

    let activity = (1.0 + total as f64).log2();
    let volume_bonus = 1.0 + (1.0 + solver.total_volume as f64).log10() / 20.0;
    let score = success_rate * reliability * activity * volume_bonus;
    (score * 10000.0).round() / 10000.0
}

fn generate_api_key() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    format!("sk_{}", hex::encode(bytes))
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_solver(wins: i64, losses: i64, volume: i64, fills: i64, failed_fills: i64) -> Solver {
        Solver {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            email: None,
            api_key: None,
            webhook_url: None,
            active: true,
            successful_trades: wins,
            failed_trades: losses,
            total_volume: volume,
            total_fills: fills,
            failed_fills,
            reputation_score: 0.0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn reputation_zero_trades() {
        let s = make_solver(0, 0, 0, 0, 0);
        assert_eq!(calculate_reputation(&s), 0.0);
    }

    #[test]
    fn reputation_perfect_record() {
        let s = make_solver(100, 0, 1_000_000, 100, 0);
        let score = calculate_reputation(&s);
        // 100% win rate, high activity, big volume → high score
        assert!(score > 5.0, "expected >5, got {score}");
    }

    #[test]
    fn reputation_penalty_for_failures() {
        let good = make_solver(80, 20, 500_000, 80, 0);
        let bad = make_solver(80, 20, 500_000, 80, 40);
        let good_score = calculate_reputation(&good);
        let bad_score = calculate_reputation(&bad);
        assert!(
            good_score > bad_score,
            "good {good_score} should beat bad {bad_score}"
        );
    }

    #[test]
    fn reputation_volume_bonus() {
        let small = make_solver(50, 10, 1_000, 50, 0);
        let big = make_solver(50, 10, 10_000_000, 50, 0);
        assert!(
            calculate_reputation(&big) > calculate_reputation(&small),
            "higher volume should yield higher score"
        );
    }

    #[test]
    fn reputation_monotonic_with_wins() {
        let few = make_solver(10, 5, 100_000, 10, 0);
        let many = make_solver(100, 50, 100_000, 100, 0);
        // Same win rate but more activity
        assert!(
            calculate_reputation(&many) > calculate_reputation(&few),
            "more activity should yield higher score"
        );
    }

    #[test]
    fn api_key_format() {
        let key = generate_api_key();
        assert!(key.starts_with("sk_"), "key should start with sk_");
        // sk_ prefix + 64 hex chars
        assert_eq!(key.len(), 3 + 64);
    }
}
