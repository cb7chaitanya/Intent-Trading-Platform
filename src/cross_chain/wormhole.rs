//! Wormhole bridge adapter.
//!
//! Wormhole uses a guardian network to produce Verified Action Approvals
//! (VAAs). The flow is: lock on source → guardians sign VAA → redeem
//! on destination with the VAA as proof.

use async_trait::async_trait;

use super::bridge::*;

/// Wormhole core bridge contract addresses by chain.
const WORMHOLE_ETHEREUM: &str = "0x98f3c9e6E3fAce36bAAd05FE09d375Ef1464288B";
const WORMHOLE_SOLANA: &str = "worm2ZoG2kUd4vFXhvjh93UUH596ayRfgQ2MgjNMTth";

pub struct WormholeBridge {
    /// Guardian RPC endpoint for fetching VAAs.
    guardian_rpc: String,
    http: reqwest::Client,
}

impl WormholeBridge {
    pub fn new(guardian_rpc: &str) -> Self {
        Self {
            guardian_rpc: guardian_rpc.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Map chain name to Wormhole chain ID.
    fn chain_id(chain: &str) -> Option<u16> {
        match chain {
            "ethereum" => Some(2),
            "solana" => Some(1),
            "polygon" => Some(5),
            "arbitrum" => Some(23),
            "base" => Some(30),
            _ => None,
        }
    }
}

#[async_trait]
impl BridgeAdapter for WormholeBridge {
    fn name(&self) -> &str {
        "wormhole"
    }

    fn supports_route(&self, source: &str, dest: &str) -> bool {
        Self::chain_id(source).is_some() && Self::chain_id(dest).is_some() && source != dest
    }

    async fn lock_funds(&self, params: &BridgeTransferParams) -> Result<LockReceipt, BridgeError> {
        let source_id = Self::chain_id(&params.source_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.source_chain.clone()))?;
        let dest_id = Self::chain_id(&params.dest_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.dest_chain.clone()))?;

        tracing::info!(
            bridge = "wormhole",
            source_chain = %params.source_chain,
            source_id,
            dest_chain = %params.dest_chain,
            dest_id,
            token = %params.token,
            amount = params.amount,
            sender = %params.sender,
            recipient = %params.recipient,
            "wormhole_lock_initiated"
        );

        // In production: call the Wormhole Token Bridge contract's
        // `transferTokens` instruction (Solana) or `wrapAndTransferETH` (EVM).
        // The tx emits a Wormhole message that guardians will sign.

        // Placeholder: simulate a successful lock
        let tx_hash = format!("0xwh_lock_{}", uuid::Uuid::new_v4());
        let message_id = format!("{source_id}/{dest_id}/{}", uuid::Uuid::new_v4());

        Ok(LockReceipt {
            tx_hash,
            message_id,
            estimated_finality_secs: self.get_bridge_time(&params.source_chain, &params.dest_chain).typical_secs,
        })
    }

    async fn verify_lock(&self, tx_hash: &str) -> Result<BridgeStatus, BridgeError> {
        tracing::debug!(bridge = "wormhole", tx_hash, "wormhole_verify_lock");

        // In production: query the guardian network for the VAA:
        //   GET {guardian_rpc}/v1/signed_vaa/{chain_id}/{emitter}/{sequence}
        // If VAA exists → InTransit; if redeemed → Completed.

        // Placeholder: treat all locks as in-transit
        Ok(BridgeStatus::InTransit {
            message_id: format!("vaa_{tx_hash}"),
        })
    }

    async fn release_funds(
        &self,
        params: &BridgeTransferParams,
        message_id: &str,
    ) -> Result<String, BridgeError> {
        tracing::info!(
            bridge = "wormhole",
            dest_chain = %params.dest_chain,
            message_id,
            recipient = %params.recipient,
            amount = params.amount,
            "wormhole_release_initiated"
        );

        // In production: submit the VAA to the destination chain's
        // Token Bridge `completeTransfer` instruction/function.

        let dest_tx = format!("0xwh_release_{}", uuid::Uuid::new_v4());
        Ok(dest_tx)
    }

    async fn estimate_bridge_fee(
        &self,
        params: &BridgeTransferParams,
    ) -> Result<BridgeFeeEstimate, BridgeError> {
        // Wormhole fees: source gas + guardian relayer fee
        // Typical: ~$0.50 per transfer on most chains
        let source_fee = match params.source_chain.as_str() {
            "solana" => 5_000,        // lamports
            _ => 50_000_000_000_000,  // ~0.00005 ETH in wei
        };

        Ok(BridgeFeeEstimate {
            source_fee,
            dest_fee: 0, // relayer covers dest gas
            protocol_fee: 0,
            total_description: "~$0.50 (gas + relayer)".into(),
        })
    }

    fn get_bridge_time(&self, source: &str, dest: &str) -> BridgeTime {
        // Wormhole needs source chain finality + guardian consensus (~13 guardians)
        let source_finality = match source {
            "solana" => 15,
            "ethereum" => 960, // 64 blocks * 15s
            _ => 120,
        };

        BridgeTime {
            min_secs: source_finality + 10,
            typical_secs: source_finality + 60,
            max_secs: source_finality + 600,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_eth_to_solana() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(bridge.supports_route("ethereum", "solana"));
        assert!(bridge.supports_route("solana", "ethereum"));
    }

    #[test]
    fn rejects_same_chain() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(!bridge.supports_route("ethereum", "ethereum"));
    }

    #[test]
    fn rejects_unknown_chain() {
        let bridge = WormholeBridge::new("http://localhost");
        assert!(!bridge.supports_route("ethereum", "avalanche"));
    }

    #[test]
    fn bridge_time_solana_source_faster() {
        let bridge = WormholeBridge::new("http://localhost");
        let sol = bridge.get_bridge_time("solana", "ethereum");
        let eth = bridge.get_bridge_time("ethereum", "solana");
        assert!(sol.typical_secs < eth.typical_secs);
    }

    #[test]
    fn chain_id_mapping() {
        assert_eq!(WormholeBridge::chain_id("ethereum"), Some(2));
        assert_eq!(WormholeBridge::chain_id("solana"), Some(1));
        assert_eq!(WormholeBridge::chain_id("unknown"), None);
    }
}
