use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Raw row stored in `solver_stats`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SolverStatsRow {
    pub solver_id: Uuid,
    pub total_auctions_entered: i64,
    pub total_auctions_won: i64,
    pub total_fills: i64,
    pub total_settled: i64,
    pub total_failed: i64,
    pub total_volume: i64,
    pub total_profit: i64,
    pub sum_latency_ms: i64,
    pub sum_slippage_bps: i64,
    pub updated_at: DateTime<Utc>,
}

/// Computed stats returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct SolverStats {
    pub solver_id: Uuid,
    pub fill_rate: f64,
    pub avg_latency_ms: f64,
    pub total_profit: i64,
    pub auction_win_rate: f64,
    pub failed_settlement_rate: f64,
    pub avg_slippage_bps: f64,
    pub total_auctions_entered: i64,
    pub total_auctions_won: i64,
    pub total_fills: i64,
    pub total_settled: i64,
    pub total_failed: i64,
    pub total_volume: i64,
    pub updated_at: DateTime<Utc>,
}

/// Leaderboard entry — solver name + computed stats.
#[derive(Debug, Clone, Serialize)]
pub struct LeaderboardEntry {
    pub solver_id: Uuid,
    pub name: String,
    pub reputation_score: f64,
    pub fill_rate: f64,
    pub avg_latency_ms: f64,
    pub total_profit: i64,
    pub auction_win_rate: f64,
    pub failed_settlement_rate: f64,
    pub avg_slippage_bps: f64,
    pub total_volume: i64,
    pub total_settled: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct LeaderboardRow {
    solver_id: Uuid,
    name: String,
    reputation_score: f64,
    total_auctions_entered: i64,
    total_auctions_won: i64,
    total_fills: i64,
    total_settled: i64,
    total_failed: i64,
    total_volume: i64,
    total_profit: i64,
    sum_latency_ms: i64,
    sum_slippage_bps: i64,
}

impl SolverStatsRow {
    pub fn compute(&self) -> SolverStats {
        let fill_rate = if self.total_fills > 0 {
            self.total_settled as f64 / self.total_fills as f64 * 100.0
        } else {
            0.0
        };

        let avg_latency_ms = if self.total_settled > 0 {
            self.sum_latency_ms as f64 / self.total_settled as f64
        } else {
            0.0
        };

        let auction_win_rate = if self.total_auctions_entered > 0 {
            self.total_auctions_won as f64 / self.total_auctions_entered as f64 * 100.0
        } else {
            0.0
        };

        let failed_settlement_rate = if self.total_fills > 0 {
            self.total_failed as f64 / self.total_fills as f64 * 100.0
        } else {
            0.0
        };

        let avg_slippage_bps = if self.total_settled > 0 {
            self.sum_slippage_bps as f64 / self.total_settled as f64
        } else {
            0.0
        };

        SolverStats {
            solver_id: self.solver_id,
            fill_rate,
            avg_latency_ms,
            total_profit: self.total_profit,
            auction_win_rate,
            failed_settlement_rate,
            avg_slippage_bps,
            total_auctions_entered: self.total_auctions_entered,
            total_auctions_won: self.total_auctions_won,
            total_fills: self.total_fills,
            total_settled: self.total_settled,
            total_failed: self.total_failed,
            total_volume: self.total_volume,
            updated_at: self.updated_at,
        }
    }
}

// ── Write operations (called from settlement) ────────────

/// Record a successful fill settlement.
///
/// * `solver_id`   — UUID of the solver
/// * `volume`      — filled quantity in base units
/// * `profit`      — solver fee earned
/// * `latency_ms`  — settlement duration in ms
/// * `slippage_bps` — (expected - actual) / expected * 10_000
pub async fn record_settled_fill(
    pool: &PgPool,
    solver_id: Uuid,
    volume: i64,
    profit: i64,
    latency_ms: i64,
    slippage_bps: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO solver_stats
            (solver_id, total_fills, total_settled, total_volume,
             total_profit, sum_latency_ms, sum_slippage_bps, updated_at)
         VALUES ($1, 1, 1, $2, $3, $4, $5, NOW())
         ON CONFLICT (solver_id) DO UPDATE SET
            total_fills     = solver_stats.total_fills     + 1,
            total_settled   = solver_stats.total_settled   + 1,
            total_volume    = solver_stats.total_volume    + $2,
            total_profit    = solver_stats.total_profit    + $3,
            sum_latency_ms  = solver_stats.sum_latency_ms  + $4,
            sum_slippage_bps = solver_stats.sum_slippage_bps + $5,
            updated_at      = NOW()",
    )
    .bind(solver_id)
    .bind(volume)
    .bind(profit)
    .bind(latency_ms)
    .bind(slippage_bps)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a failed fill settlement.
pub async fn record_failed_fill(
    pool: &PgPool,
    solver_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO solver_stats (solver_id, total_fills, total_failed, updated_at)
         VALUES ($1, 1, 1, NOW())
         ON CONFLICT (solver_id) DO UPDATE SET
            total_fills  = solver_stats.total_fills  + 1,
            total_failed = solver_stats.total_failed + 1,
            updated_at   = NOW()",
    )
    .bind(solver_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Increment auction counters (entered and optionally won).
pub async fn record_auction_result(
    pool: &PgPool,
    solver_id: Uuid,
    won: bool,
) -> Result<(), sqlx::Error> {
    let won_inc: i64 = if won { 1 } else { 0 };
    sqlx::query(
        "INSERT INTO solver_stats
            (solver_id, total_auctions_entered, total_auctions_won, updated_at)
         VALUES ($1, 1, $2, NOW())
         ON CONFLICT (solver_id) DO UPDATE SET
            total_auctions_entered = solver_stats.total_auctions_entered + 1,
            total_auctions_won     = solver_stats.total_auctions_won     + $2,
            updated_at             = NOW()",
    )
    .bind(solver_id)
    .bind(won_inc)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Read operations ──────────────────────────────────────

/// Get stats for one solver.
pub async fn get_stats(pool: &PgPool, solver_id: Uuid) -> Result<Option<SolverStats>, sqlx::Error> {
    let row = sqlx::query_as::<_, SolverStatsRow>(
        "SELECT * FROM solver_stats WHERE solver_id = $1",
    )
    .bind(solver_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.compute()))
}

/// Leaderboard: top solvers ranked by reputation, enriched with stats.
pub async fn get_leaderboard(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<LeaderboardEntry>, sqlx::Error> {
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query_as::<_, LeaderboardRow>(
        "SELECT s.id AS solver_id, s.name, s.reputation_score,
                COALESCE(st.total_auctions_entered, 0) AS total_auctions_entered,
                COALESCE(st.total_auctions_won, 0) AS total_auctions_won,
                COALESCE(st.total_fills, 0) AS total_fills,
                COALESCE(st.total_settled, 0) AS total_settled,
                COALESCE(st.total_failed, 0) AS total_failed,
                COALESCE(st.total_volume, 0) AS total_volume,
                COALESCE(st.total_profit, 0) AS total_profit,
                COALESCE(st.sum_latency_ms, 0) AS sum_latency_ms,
                COALESCE(st.sum_slippage_bps, 0) AS sum_slippage_bps
         FROM solvers s
         LEFT JOIN solver_stats st ON st.solver_id = s.id
         WHERE s.active = TRUE
         ORDER BY s.reputation_score DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| {
        let total_fills = r.total_fills.max(1);
        let total_settled = r.total_settled.max(1);
        let auctions_entered = r.total_auctions_entered.max(1);
        LeaderboardEntry {
            solver_id: r.solver_id,
            name: r.name,
            reputation_score: r.reputation_score,
            fill_rate: r.total_settled as f64 / total_fills as f64 * 100.0,
            avg_latency_ms: r.sum_latency_ms as f64 / total_settled as f64,
            total_profit: r.total_profit,
            auction_win_rate: r.total_auctions_won as f64 / auctions_entered as f64 * 100.0,
            failed_settlement_rate: r.total_failed as f64 / total_fills as f64 * 100.0,
            avg_slippage_bps: r.sum_slippage_bps as f64 / total_settled as f64,
            total_volume: r.total_volume,
            total_settled: r.total_settled,
        }
    }).collect())
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn row(settled: i64, failed: i64, fills: i64, entered: i64, won: i64, vol: i64, profit: i64, lat: i64, slip: i64) -> SolverStatsRow {
        SolverStatsRow {
            solver_id: Uuid::new_v4(),
            total_auctions_entered: entered,
            total_auctions_won: won,
            total_fills: fills,
            total_settled: settled,
            total_failed: failed,
            total_volume: vol,
            total_profit: profit,
            sum_latency_ms: lat,
            sum_slippage_bps: slip,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn stats_zero() {
        let s = row(0, 0, 0, 0, 0, 0, 0, 0, 0).compute();
        assert_eq!(s.fill_rate, 0.0);
        assert_eq!(s.avg_latency_ms, 0.0);
        assert_eq!(s.auction_win_rate, 0.0);
        assert_eq!(s.failed_settlement_rate, 0.0);
        assert_eq!(s.avg_slippage_bps, 0.0);
    }

    #[test]
    fn stats_perfect_solver() {
        let s = row(100, 0, 100, 200, 100, 5_000_000, 50_000, 5000, 1000).compute();
        assert_eq!(s.fill_rate, 100.0);
        assert_eq!(s.avg_latency_ms, 50.0);        // 5000 / 100
        assert_eq!(s.auction_win_rate, 50.0);       // 100 / 200
        assert_eq!(s.failed_settlement_rate, 0.0);
        assert_eq!(s.avg_slippage_bps, 10.0);       // 1000 / 100
        assert_eq!(s.total_profit, 50_000);
    }

    #[test]
    fn stats_with_failures() {
        let s = row(80, 20, 100, 300, 100, 1_000_000, 10_000, 8000, 2000).compute();
        assert_eq!(s.fill_rate, 80.0);
        assert_eq!(s.failed_settlement_rate, 20.0);
        assert_eq!(s.avg_latency_ms, 100.0);        // 8000 / 80
        assert_eq!(s.avg_slippage_bps, 25.0);        // 2000 / 80
    }

    #[test]
    fn stats_auction_win_rate() {
        let s = row(50, 0, 50, 500, 50, 100_000, 1000, 2500, 500).compute();
        assert_eq!(s.auction_win_rate, 10.0);        // 50 / 500
    }
}
