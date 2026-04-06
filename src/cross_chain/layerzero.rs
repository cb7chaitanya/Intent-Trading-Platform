//! LayerZero bridge adapter.
//!
//! LayerZero uses a configurable messaging layer with DVNs (Decentralized
//! Verifier Networks) and executors. The OFT (Omnichain Fungible Token)
//! standard handles token transfers via `send` on source → `lzReceive`
//! on destination.

use async_trait::async_trait;

use super::bridge::*;

pub struct LayerZeroBridge {
    /// LayerZero Scan API endpoint for message tracking.
    scan_api: String,
    http: reqwest::Client,
}

impl LayerZeroBridge {
    pub fn new(scan_api: &str) -> Self {
        Self {
            scan_api: scan_api.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Map chain name to LayerZero v2 endpoint ID.
    fn endpoint_id(chain: &str) -> Option<u32> {
        match chain {
            "ethereum" => Some(30101),
            "solana" => Some(30168),
            "polygon" => Some(30109),
            "arbitrum" => Some(30110),
            "base" => Some(30184),
            _ => None,
        }
    }
}

#[async_trait]
impl BridgeAdapter for LayerZeroBridge {
    fn name(&self) -> &str {
        "layerzero"
    }

    fn supports_route(&self, source: &str, dest: &str) -> bool {
        Self::endpoint_id(source).is_some() && Self::endpoint_id(dest).is_some() && source != dest
    }

    async fn lock_funds(&self, params: &BridgeTransferParams) -> Result<LockReceipt, BridgeError> {
        let src_eid = Self::endpoint_id(&params.source_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.source_chain.clone()))?;
        let dst_eid = Self::endpoint_id(&params.dest_chain)
            .ok_or_else(|| BridgeError::UnsupportedRoute(params.dest_chain.clone()))?;

        tracing::info!(
            bridge = "layerzero",
            source_chain = %params.source_chain,
            src_eid,
            dest_chain = %params.dest_chain,
            dst_eid,
            token = %params.token,
            amount = params.amount,
            "layerzero_send_initiated"
        );

        // In production: call the OFT contract's `send()` function which
        // takes (dstEid, to, amountLD, minAmountLD, extraOptions, ...).
        // The endpoint emits a Packet event that DVNs will verify.

        let tx_hash = format!("0xlz_send_{}", uuid::Uuid::new_v4());
        let message_id = format!("lz_{src_eid}_{dst_eid}_{}", uuid::Uuid::new_v4());

        Ok(LockReceipt {
            tx_hash,
            message_id,
            estimated_finality_secs: self.get_bridge_time(&params.source_chain, &params.dest_chain).typical_secs,
        })
    }

    async fn verify_lock(&self, tx_hash: &str) -> Result<BridgeStatus, BridgeError> {
        tracing::debug!(bridge = "layerzero", tx_hash, "layerzero_verify");

        // In production: query LayerZero Scan API:
        //   GET {scan_api}/v1/messages/tx/{tx_hash}
        // Check status: INFLIGHT, DELIVERED, FAILED.

        Ok(BridgeStatus::InTransit {
            message_id: format!("lz_msg_{tx_hash}"),
        })
    }

    async fn release_funds(
        &self,
        params: &BridgeTransferParams,
        message_id: &str,
    ) -> Result<String, BridgeError> {
        tracing::info!(
            bridge = "layerzero",
            dest_chain = %params.dest_chain,
            message_id,
            recipient = %params.recipient,
            amount = params.amount,
            "layerzero_receive_initiated"
        );

        // In production: LayerZero's executor automatically calls
        // `lzReceive` on the destination OFT contract. No manual
        // claim needed for standard OFT transfers. If using a custom
        // compose pattern, we'd need to submit a claim tx.

        let dest_tx = format!("0xlz_receive_{}", uuid::Uuid::new_v4());
        Ok(dest_tx)
    }

    async fn estimate_bridge_fee(
        &self,
        params: &BridgeTransferParams,
    ) -> Result<BridgeFeeEstimate, BridgeError> {
        // In production: call OFT.quoteSend() on-chain to get exact fee.
        // LayerZero fees = DVN fee + executor fee (~$0.10-$1.00).
        let source_fee = match params.source_chain.as_str() {
            "solana" => 10_000,       // lamports
            _ => 100_000_000_000_000, // ~0.0001 ETH in wei
        };

        Ok(BridgeFeeEstimate {
            source_fee,
            dest_fee: 0, // executor covers destination gas
            protocol_fee: 0,
            total_description: "~$0.10–$1.00 (DVN + executor)".into(),
        })
    }

    fn get_bridge_time(&self, source: &str, _dest: &str) -> BridgeTime {
        // LayerZero v2 is fast: source finality + DVN verification (~10-30s)
        let source_finality = match source {
            "solana" => 5,
            "ethereum" => 780, // ~52 blocks for LayerZero default
            _ => 60,
        };

        BridgeTime {
            min_secs: source_finality + 5,
            typical_secs: source_finality + 30,
            max_secs: source_finality + 300,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_routes() {
        let bridge = LayerZeroBridge::new("http://localhost");
        assert!(bridge.supports_route("ethereum", "solana"));
        assert!(bridge.supports_route("polygon", "arbitrum"));
        assert!(!bridge.supports_route("ethereum", "ethereum"));
        assert!(!bridge.supports_route("ethereum", "cosmos"));
    }

    #[test]
    fn endpoint_ids() {
        assert_eq!(LayerZeroBridge::endpoint_id("ethereum"), Some(30101));
        assert_eq!(LayerZeroBridge::endpoint_id("solana"), Some(30168));
        assert_eq!(LayerZeroBridge::endpoint_id("unknown"), None);
    }

    #[test]
    fn layerzero_faster_than_wormhole_for_solana() {
        let lz = LayerZeroBridge::new("http://localhost");
        let time = lz.get_bridge_time("solana", "ethereum");
        // Solana source: 5s finality + 30s DVN = 35s typical
        assert!(time.typical_secs < 60);
    }
}
