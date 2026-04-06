use std::sync::Arc;

use super::bridge::{BridgeAdapter, BridgeError};

/// Registry of bridge adapters. Selects the best bridge for a given route.
pub struct BridgeRegistry {
    bridges: Vec<Arc<dyn BridgeAdapter>>,
}

impl BridgeRegistry {
    pub fn new() -> Self {
        Self {
            bridges: Vec::new(),
        }
    }

    pub fn register(&mut self, bridge: Arc<dyn BridgeAdapter>) {
        self.bridges.push(bridge);
    }

    /// Find a bridge that supports the given route.
    /// Returns the first registered bridge that supports the pair.
    /// In production, could rank by fee or speed.
    pub fn find(
        &self,
        source_chain: &str,
        dest_chain: &str,
    ) -> Result<&Arc<dyn BridgeAdapter>, BridgeError> {
        self.bridges
            .iter()
            .find(|b| b.supports_route(source_chain, dest_chain))
            .ok_or_else(|| {
                BridgeError::UnsupportedRoute(format!("{source_chain} -> {dest_chain}"))
            })
    }

    pub fn list_bridges(&self) -> Vec<&str> {
        self.bridges.iter().map(|b| b.name()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_chain::layerzero::LayerZeroBridge;
    use crate::cross_chain::wormhole::WormholeBridge;

    #[test]
    fn find_bridge_for_route() {
        let mut reg = BridgeRegistry::new();
        reg.register(Arc::new(WormholeBridge::new("http://localhost")));
        reg.register(Arc::new(LayerZeroBridge::new("http://localhost")));

        let b = reg.find("ethereum", "solana").unwrap();
        // First registered wins — wormhole in this case
        assert_eq!(b.name(), "wormhole");
    }

    #[test]
    fn no_bridge_for_unknown_route() {
        let reg = BridgeRegistry::new();
        assert!(reg.find("ethereum", "cosmos").is_err());
    }

    #[test]
    fn lists_bridges() {
        let mut reg = BridgeRegistry::new();
        reg.register(Arc::new(WormholeBridge::new("http://localhost")));
        reg.register(Arc::new(LayerZeroBridge::new("http://localhost")));
        assert_eq!(reg.list_bridges(), vec!["wormhole", "layerzero"]);
    }
}
