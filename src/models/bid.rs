use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SolverBid {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub solver_id: String,
    pub amount_out: i64,
    pub fee: i64,
    pub timestamp: i64,
}

impl SolverBid {
    pub fn new(
        intent_id: Uuid,
        solver_id: String,
        amount_out: u64,
        fee: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            solver_id,
            amount_out: amount_out as i64,
            fee: fee as i64,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_bid_stores_correct_values() {
        let intent_id = Uuid::new_v4();
        let bid = SolverBid::new(intent_id, "solver-1".into(), 5000, 25);

        assert_eq!(bid.intent_id, intent_id);
        assert_eq!(bid.solver_id, "solver-1");
        assert_eq!(bid.amount_out, 5000);
        assert_eq!(bid.fee, 25);
        assert!(bid.timestamp > 0);
    }

    #[test]
    fn bid_net_value_calculation() {
        let bid = SolverBid::new(Uuid::new_v4(), "s".into(), 1000, 50);
        let net = bid.amount_out - bid.fee;
        assert_eq!(net, 950);
    }

    #[test]
    fn bid_ordering_by_net_value() {
        let iid = Uuid::new_v4();
        let bid_a = SolverBid::new(iid, "a".into(), 1000, 100); // net 900
        let bid_b = SolverBid::new(iid, "b".into(), 950, 10);   // net 940

        let best = [bid_a, bid_b]
            .into_iter()
            .max_by(|a, b| {
                let na = a.amount_out - a.fee;
                let nb = b.amount_out - b.fee;
                na.cmp(&nb)
            })
            .unwrap();

        assert_eq!(best.solver_id, "b");
    }
}
