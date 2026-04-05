use std::collections::HashMap;
use std::sync::Arc;

use super::chain::{ChainAdapter, ChainError};

/// Registry of chain adapters keyed by chain name.
///
/// Created once at startup and shared via Arc across the settlement engine,
/// wallet service, and confirmation worker.
pub struct ChainRegistry {
    adapters: HashMap<String, Arc<dyn ChainAdapter>>,
}

impl ChainRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register an adapter for a chain. Overwrites any previous adapter for
    /// the same chain name.
    pub fn register(&mut self, adapter: Arc<dyn ChainAdapter>) {
        self.adapters
            .insert(adapter.chain_name().to_string(), adapter);
    }

    /// Get the adapter for a chain, or error if unsupported.
    pub fn get(&self, chain: &str) -> Result<&Arc<dyn ChainAdapter>, ChainError> {
        self.adapters
            .get(chain)
            .ok_or_else(|| ChainError::Unsupported(format!("No adapter for chain: {chain}")))
    }

    /// List all registered chain names.
    pub fn chains(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::chain::*;
    use async_trait::async_trait;

    struct MockAdapter;

    #[async_trait]
    impl ChainAdapter for MockAdapter {
        fn chain_name(&self) -> &str { "mock" }
        fn required_confirmations(&self) -> u32 { 1 }
        async fn send_transaction(&self, _: &SignedTx) -> Result<String, ChainError> {
            Ok("0xabc".into())
        }
        async fn get_transaction_status(&self, _: &str) -> Result<TxState, ChainError> {
            Ok(TxState::Pending)
        }
        async fn estimate_fees(&self, _: &SettlementData) -> Result<FeeEstimate, ChainError> {
            Ok(FeeEstimate { base_fee: 1, priority_fee: 0, total: 1, unit: "test".into() })
        }
        async fn get_balance(&self, _: &str, _: &str) -> Result<u64, ChainError> {
            Ok(1000)
        }
        async fn build_settlement_tx(&self, _: &SettlementData) -> Result<UnsignedTx, ChainError> {
            Ok(UnsignedTx { chain: "mock".into(), data: vec![1, 2, 3] })
        }
        fn sign_transaction(&self, _: &UnsignedTx, _: &[u8; 32]) -> Result<SignedTx, ChainError> {
            Ok(SignedTx { chain: "mock".into(), data: vec![4, 5, 6] })
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = ChainRegistry::new();
        reg.register(Arc::new(MockAdapter));
        assert!(reg.get("mock").is_ok());
        assert!(reg.get("unknown").is_err());
    }

    #[test]
    fn lists_chains() {
        let mut reg = ChainRegistry::new();
        reg.register(Arc::new(MockAdapter));
        assert_eq!(reg.chains(), vec!["mock"]);
    }
}
