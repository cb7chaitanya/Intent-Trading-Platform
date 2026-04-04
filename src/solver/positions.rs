use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SolverPosition {
    pub solver_id: String,
    pub asset: String,
    pub position: i64,
    pub avg_entry_price: i64,
    pub realized_pnl: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PositionPnl {
    pub solver_id: String,
    pub asset: String,
    pub position: i64,
    pub avg_entry_price: i64,
    pub current_price: i64,
    pub unrealized_pnl: i64,
    pub realized_pnl: i64,
    pub total_pnl: i64,
}

pub struct PositionTracker {
    pool: PgPool,
}

impl PositionTracker {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Load all positions for a solver.
    pub async fn load_positions(&self, solver_id: &str) -> Result<Vec<SolverPosition>, sqlx::Error> {
        sqlx::query_as::<_, SolverPosition>(
            "SELECT * FROM solver_positions WHERE solver_id = $1 ORDER BY asset",
        )
        .bind(solver_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Get current position size for an asset.
    pub async fn get_position(&self, solver_id: &str, asset: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT position FROM solver_positions WHERE solver_id = $1 AND asset = $2",
        )
        .bind(solver_id)
        .bind(asset)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
    }

    /// Get total absolute position across all assets.
    pub async fn get_total_exposure(&self, solver_id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(ABS(position)), 0) FROM solver_positions WHERE solver_id = $1",
        )
        .bind(solver_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
    }

    /// Check if adding `amount` to an asset would exceed the max position limit.
    pub async fn check_limit(&self, solver_id: &str, amount: i64, max_position: i64) -> bool {
        let exposure = self.get_total_exposure(solver_id).await;
        exposure + amount.abs() <= max_position
    }

    /// Record a fill: increase position and update average entry price.
    pub async fn record_fill(
        &self,
        solver_id: &str,
        asset: &str,
        qty: i64,
        price: i64,
    ) -> Result<SolverPosition, sqlx::Error> {
        let now = Utc::now();

        // Upsert with weighted average price calculation
        let existing = sqlx::query_as::<_, SolverPosition>(
            "SELECT * FROM solver_positions WHERE solver_id = $1 AND asset = $2",
        )
        .bind(solver_id)
        .bind(asset)
        .fetch_optional(&self.pool)
        .await?;

        match existing {
            Some(pos) => {
                let new_position = pos.position + qty;
                let new_avg = if new_position != 0 && (pos.position.signum() == qty.signum() || pos.position == 0) {
                    // Adding to position — weighted average
                    let total_cost = pos.avg_entry_price * pos.position.abs() + price * qty.abs();
                    total_cost / new_position.abs()
                } else if new_position == 0 {
                    // Flat — realize PnL
                    0
                } else {
                    // Reducing or flipping — keep old avg for remaining, realize PnL on closed portion
                    pos.avg_entry_price
                };

                // Realized PnL from closed portion
                let closed_qty = if pos.position.signum() != qty.signum() {
                    qty.abs().min(pos.position.abs())
                } else {
                    0
                };
                let realized = closed_qty * (price - pos.avg_entry_price);

                sqlx::query(
                    "UPDATE solver_positions
                     SET position = $1, avg_entry_price = $2, realized_pnl = realized_pnl + $3, updated_at = $4
                     WHERE solver_id = $5 AND asset = $6",
                )
                .bind(new_position)
                .bind(new_avg)
                .bind(realized)
                .bind(now)
                .bind(solver_id)
                .bind(asset)
                .execute(&self.pool)
                .await?;

                Ok(SolverPosition {
                    solver_id: solver_id.to_string(),
                    asset: asset.to_string(),
                    position: new_position,
                    avg_entry_price: new_avg,
                    realized_pnl: pos.realized_pnl + realized,
                    updated_at: now,
                })
            }
            None => {
                let pos = SolverPosition {
                    solver_id: solver_id.to_string(),
                    asset: asset.to_string(),
                    position: qty,
                    avg_entry_price: price,
                    realized_pnl: 0,
                    updated_at: now,
                };

                sqlx::query(
                    "INSERT INTO solver_positions (solver_id, asset, position, avg_entry_price, realized_pnl, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(&pos.solver_id)
                .bind(&pos.asset)
                .bind(pos.position)
                .bind(pos.avg_entry_price)
                .bind(pos.realized_pnl)
                .bind(pos.updated_at)
                .execute(&self.pool)
                .await?;

                Ok(pos)
            }
        }
    }

    /// Calculate PnL for a position given the current market price.
    pub fn calculate_pnl(pos: &SolverPosition, current_price: i64) -> PositionPnl {
        let unrealized = if pos.position != 0 {
            pos.position * (current_price - pos.avg_entry_price)
        } else {
            0
        };

        PositionPnl {
            solver_id: pos.solver_id.clone(),
            asset: pos.asset.clone(),
            position: pos.position,
            avg_entry_price: pos.avg_entry_price,
            current_price,
            unrealized_pnl: unrealized,
            realized_pnl: pos.realized_pnl,
            total_pnl: unrealized + pos.realized_pnl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnl_long_position_profit() {
        let pos = SolverPosition {
            solver_id: "s1".into(), asset: "ETH".into(),
            position: 10, avg_entry_price: 3000,
            realized_pnl: 0, updated_at: Utc::now(),
        };
        let pnl = PositionTracker::calculate_pnl(&pos, 3500);
        assert_eq!(pnl.unrealized_pnl, 10 * 500); // 5000
        assert_eq!(pnl.total_pnl, 5000);
    }

    #[test]
    fn pnl_long_position_loss() {
        let pos = SolverPosition {
            solver_id: "s1".into(), asset: "ETH".into(),
            position: 10, avg_entry_price: 3000,
            realized_pnl: 0, updated_at: Utc::now(),
        };
        let pnl = PositionTracker::calculate_pnl(&pos, 2800);
        assert_eq!(pnl.unrealized_pnl, 10 * -200); // -2000
    }

    #[test]
    fn pnl_flat_position() {
        let pos = SolverPosition {
            solver_id: "s1".into(), asset: "ETH".into(),
            position: 0, avg_entry_price: 0,
            realized_pnl: 500, updated_at: Utc::now(),
        };
        let pnl = PositionTracker::calculate_pnl(&pos, 3500);
        assert_eq!(pnl.unrealized_pnl, 0);
        assert_eq!(pnl.total_pnl, 500);
    }

    #[test]
    fn pnl_with_realized() {
        let pos = SolverPosition {
            solver_id: "s1".into(), asset: "BTC".into(),
            position: 5, avg_entry_price: 60000,
            realized_pnl: 10000, updated_at: Utc::now(),
        };
        let pnl = PositionTracker::calculate_pnl(&pos, 65000);
        assert_eq!(pnl.unrealized_pnl, 5 * 5000); // 25000
        assert_eq!(pnl.total_pnl, 25000 + 10000); // 35000
    }
}
