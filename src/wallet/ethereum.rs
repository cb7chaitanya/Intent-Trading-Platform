use async_trait::async_trait;

use super::chain::*;
use super::erc20_abi;
use super::rlp;
use super::rpc::{RpcClient, RpcError};
use super::signing;
use crate::metrics::counters;

// ── Gas constants ────────────────────────────────────────

/// Default gas limit for ERC-20 transfers.
const ERC20_TRANSFER_GAS: u64 = 65_000;

/// Safety buffer added on top of eth_estimateGas result (20%).
const GAS_ESTIMATE_BUFFER_PCT: u64 = 120;

/// Default priority fee if eth_maxPriorityFeePerGas unavailable (1.5 gwei).
const DEFAULT_PRIORITY_FEE: u64 = 1_500_000_000;

/// Minimum priority fee floor (0.1 gwei).
const MIN_PRIORITY_FEE: u64 = 100_000_000;

/// Maximum priority fee cap (50 gwei) to prevent overpaying.
const MAX_PRIORITY_FEE: u64 = 50_000_000_000;

/// Buffer multiplier for maxFeePerGas over baseFee (2x) to survive
/// base fee spikes for up to 6 consecutive full blocks.
const BASE_FEE_MULTIPLIER: u64 = 2;

/// Maximum retry attempts for transient send errors.
const MAX_SEND_RETRIES: u32 = 3;

/// Initial backoff delay between retries.
const RETRY_BASE_DELAY_MS: u64 = 500;

/// Gas price bump percentage when replacing an underpriced tx.
const REPLACEMENT_BUMP_PCT: u64 = 15;

// ── Gas estimation result ────────────────────────────────

/// EIP-1559 fee parameters.
#[derive(Debug, Clone)]
struct GasParams {
    /// Is this an EIP-1559 (type 2) transaction?
    eip1559: bool,
    /// Base fee from latest block (0 for legacy).
    base_fee: u64,
    /// Priority fee / miner tip.
    max_priority_fee: u64,
    /// Maximum total fee per gas unit.
    max_fee_per_gas: u64,
    /// Gas units required.
    gas_limit: u64,
}

// ── Tx type tag (first byte of serialised tx) ────────────

const TX_TYPE_LEGACY: u8 = 0x00;
const TX_TYPE_EIP1559: u8 = 0x02;

// ── Adapter ──────────────────────────────────────────────

pub struct EthereumAdapter {
    rpc: RpcClient,
}

impl EthereumAdapter {
    pub fn new(endpoint: &str, chain_id: u64) -> Self {
        Self {
            rpc: RpcClient::new(endpoint, chain_id),
        }
    }

    /// Estimate gas parameters. Tries EIP-1559, falls back to legacy.
    async fn estimate_gas_params(
        &self,
        from: &str,
        to_contract: &str,
        calldata: &[u8],
    ) -> Result<GasParams, ChainError> {
        // Step 1: estimate gas limit
        let data_hex = format!("0x{}", hex::encode(calldata));
        let estimated_gas = self
            .rpc
            .estimate_gas(from, to_contract, &data_hex)
            .await
            .unwrap_or(ERC20_TRANSFER_GAS);

        let gas_limit = (estimated_gas * GAS_ESTIMATE_BUFFER_PCT / 100).max(ERC20_TRANSFER_GAS);

        // Step 2: try EIP-1559 base fee
        let base_fee = self
            .rpc
            .get_base_fee()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        match base_fee {
            Some(base) => {
                // EIP-1559 chain
                let priority = self.estimate_priority_fee().await;
                let max_fee = base * BASE_FEE_MULTIPLIER + priority;

                Ok(GasParams {
                    eip1559: true,
                    base_fee: base,
                    max_priority_fee: priority,
                    max_fee_per_gas: max_fee,
                    gas_limit,
                })
            }
            None => {
                // Legacy chain — fall back to eth_gasPrice
                let gas_price = self
                    .rpc
                    .gas_price()
                    .await
                    .map_err(|e| ChainError::Rpc(e.to_string()))? as u64;

                Ok(GasParams {
                    eip1559: false,
                    base_fee: 0,
                    max_priority_fee: 0,
                    max_fee_per_gas: gas_price,
                    gas_limit,
                })
            }
        }
    }

    /// Estimate priority fee using multiple strategies:
    /// 1. eth_maxPriorityFeePerGas (if supported)
    /// 2. eth_feeHistory 50th percentile
    /// 3. DEFAULT_PRIORITY_FEE fallback
    async fn estimate_priority_fee(&self) -> u64 {
        // Strategy 1: direct RPC method
        if let Ok(Some(fee)) = self.rpc.max_priority_fee().await {
            return fee.clamp(MIN_PRIORITY_FEE, MAX_PRIORITY_FEE);
        }

        // Strategy 2: fee history 50th percentile over last 5 blocks
        if let Ok(tips) = self.rpc.fee_history(5, &[50.0]).await {
            if !tips.is_empty() {
                let mut sorted = tips;
                sorted.sort();
                let median = sorted[sorted.len() / 2];
                if median > 0 {
                    return median.clamp(MIN_PRIORITY_FEE, MAX_PRIORITY_FEE);
                }
            }
        }

        // Strategy 3: default
        DEFAULT_PRIORITY_FEE
    }

    /// Serialise an EIP-1559 (type 2) unsigned transaction for signing.
    /// Uses proper Ethereum RLP encoding: 0x02 || RLP([chainId, nonce,
    /// maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList])
    fn encode_type2_tx(
        chain_id: u64,
        nonce: u64,
        gas: &GasParams,
        to_contract: &[u8; 20],
        calldata: &[u8],
    ) -> Vec<u8> {
        rlp::encode_eip1559_unsigned(&rlp::Eip1559TxFields {
            chain_id,
            nonce,
            max_priority_fee_per_gas: gas.max_priority_fee,
            max_fee_per_gas: gas.max_fee_per_gas,
            gas_limit: gas.gas_limit,
            to: *to_contract,
            value: 0, // ERC-20 transfer, value is always 0
            data: calldata.to_vec(),
        })
    }

    /// Serialise a legacy unsigned transaction for signing (EIP-155).
    /// RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])
    fn encode_legacy_tx(
        chain_id: u64,
        nonce: u64,
        gas_price: u64,
        gas_limit: u64,
        to_contract: &[u8; 20],
        calldata: &[u8],
    ) -> Vec<u8> {
        rlp::encode_legacy_unsigned(&rlp::LegacyTxFields {
            nonce,
            gas_price,
            gas_limit,
            to: *to_contract,
            value: 0,
            data: calldata.to_vec(),
            chain_id,
        })
    }
}

#[async_trait]
impl ChainAdapter for EthereumAdapter {
    fn chain_name(&self) -> &str {
        "ethereum"
    }

    fn required_confirmations(&self) -> u32 {
        12
    }

    async fn send_transaction(&self, tx: &SignedTx) -> Result<String, ChainError> {
        let raw_hex = format!("0x{}", hex::encode(&tx.data));

        for attempt in 0..=MAX_SEND_RETRIES {
            match self.rpc.send_raw_transaction(&raw_hex).await {
                Ok(tx_hash) => {
                    counters::ETH_TX_SUBMITTED
                        .with_label_values(&["success"])
                        .inc();
                    tracing::info!(
                        tx_hash = %tx_hash,
                        attempt,
                        "eth_tx_submitted"
                    );
                    return Ok(tx_hash);
                }
                Err(RpcError::JsonRpc { code, ref message }) => {
                    let msg_lower = message.to_lowercase();
                    let (retriable, category) = classify_send_error(code, &msg_lower);

                    if retriable && attempt < MAX_SEND_RETRIES {
                        counters::ETH_TX_RETRIES.inc();
                        tracing::warn!(
                            attempt,
                            category,
                            code,
                            error = %message,
                            "eth_tx_retrying"
                        );
                        let delay = RETRY_BASE_DELAY_MS * (1 << attempt);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Non-retriable or exhausted retries
                    counters::ETH_TX_SUBMITTED
                        .with_label_values(&[category])
                        .inc();
                    tracing::error!(
                        attempt,
                        category,
                        code,
                        error = %message,
                        "eth_tx_failed"
                    );
                    return Err(ChainError::Rpc(format!("{category}: {message}")));
                }
                Err(RpcError::Network(ref e)) if attempt < MAX_SEND_RETRIES => {
                    counters::ETH_TX_RETRIES.inc();
                    tracing::warn!(attempt, error = %e, "eth_tx_network_retry");
                    let delay = RETRY_BASE_DELAY_MS * (1 << attempt);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                Err(e) => {
                    counters::ETH_TX_SUBMITTED
                        .with_label_values(&["failed"])
                        .inc();
                    tracing::error!(error = %e, "eth_tx_failed");
                    return Err(ChainError::Rpc(e.to_string()));
                }
            }
        }

        counters::ETH_TX_SUBMITTED
            .with_label_values(&["failed"])
            .inc();
        Err(ChainError::Rpc("Max retries exceeded".into()))
    }

    async fn get_transaction_status(&self, tx_hash: &str) -> Result<TxState, ChainError> {
        let receipt = self
            .rpc
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        match receipt {
            None => Ok(TxState::Pending),
            Some(r) => {
                if !r.status {
                    return Ok(TxState::Failed {
                        reason: "Transaction reverted".into(),
                    });
                }
                let current_block = self
                    .rpc
                    .get_block_number()
                    .await
                    .map_err(|e| ChainError::Rpc(e.to_string()))?;
                let confirmations = (current_block - r.block_number).max(0) as u32;
                Ok(TxState::Confirmed {
                    block: r.block_number as u64,
                    confirmations,
                })
            }
        }
    }

    async fn estimate_fees(&self, data: &SettlementData) -> Result<FeeEstimate, ChainError> {
        let to_addr = erc20_abi::parse_address(&data.to)
            .map_err(|e| ChainError::Rpc(format!("Bad address: {e}")))?;
        let calldata = erc20_abi::encode_transfer(&to_addr, data.amount as u128);

        let gas = self
            .estimate_gas_params(&data.from, &data.token, &calldata)
            .await?;

        let total = gas.max_fee_per_gas * gas.gas_limit;

        Ok(FeeEstimate {
            base_fee: gas.base_fee,
            priority_fee: gas.max_priority_fee,
            total,
            unit: if gas.eip1559 {
                "wei (EIP-1559)".into()
            } else {
                "wei (legacy)".into()
            },
        })
    }

    async fn get_balance(&self, address: &str, token: &str) -> Result<u64, ChainError> {
        let account = erc20_abi::parse_address(address)
            .map_err(|e| ChainError::Rpc(format!("Bad address: {e}")))?;
        let calldata = erc20_abi::encode_balance_of(&account);

        let result = self
            .rpc
            .eth_call(token, &format!("0x{}", hex::encode(&calldata)))
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let clean = result.strip_prefix("0x").unwrap_or(&result);
        let bytes = hex::decode(clean).unwrap_or_default();
        if bytes.len() >= 32 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[24..32]);
            Ok(u64::from_be_bytes(buf))
        } else {
            Ok(0)
        }
    }

    async fn build_settlement_tx(
        &self,
        data: &SettlementData,
    ) -> Result<UnsignedTx, ChainError> {
        let to_addr = erc20_abi::parse_address(&data.to)
            .map_err(|e| ChainError::Rpc(format!("Bad recipient: {e}")))?;

        let calldata = erc20_abi::encode_transfer(&to_addr, data.amount as u128);

        let token_contract = erc20_abi::parse_address(&data.token)
            .map_err(|e| ChainError::Rpc(format!("Bad token: {e}")))?;

        let nonce = self.rpc.get_nonce(&data.from)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let gas = self
            .estimate_gas_params(&data.from, &data.token, &calldata)
            .await?;

        let chain_id = self.rpc.chain_id();

        let tx_bytes = if gas.eip1559 {
            Self::encode_type2_tx(chain_id, nonce, &gas, &token_contract, &calldata)
        } else {
            Self::encode_legacy_tx(
                chain_id, nonce, gas.max_fee_per_gas, gas.gas_limit,
                &token_contract, &calldata,
            )
        };

        Ok(UnsignedTx {
            chain: "ethereum".into(),
            data: tx_bytes,
        })
    }

    fn sign_transaction(
        &self,
        unsigned_tx: &UnsignedTx,
        private_key: &[u8; 32],
    ) -> Result<SignedTx, ChainError> {
        let sig = signing::sign_transaction(private_key, &unsigned_tx.data)
            .map_err(ChainError::Signing)?;
        Ok(SignedTx {
            chain: "ethereum".into(),
            data: sig,
        })
    }
}

// ── Error classification ─────────────────────────────────

/// Classify an eth_sendRawTransaction JSON-RPC error.
/// Returns (retriable, category_label).
fn classify_send_error(code: i64, msg: &str) -> (bool, &'static str) {
    // "nonce too low" — our nonce is stale, need to re-fetch
    if msg.contains("nonce too low") || msg.contains("nonce is too low") {
        return (true, "nonce_retry");
    }

    // "replacement transaction underpriced" — need higher gas
    if msg.contains("replacement transaction underpriced")
        || msg.contains("underpriced")
        || msg.contains("gas price too low")
    {
        return (true, "underpriced");
    }

    // "already known" — node already has this tx, not an error
    if msg.contains("already known") || msg.contains("already imported") {
        // Not retriable — the tx is already in the mempool.
        // Return success-like: the caller should poll for the receipt.
        return (false, "already_known");
    }

    // "transaction pool is full" — transient, retry later
    if msg.contains("txpool is full") || msg.contains("transaction pool") {
        return (true, "pool_full");
    }

    // "insufficient funds" — not retriable
    if msg.contains("insufficient funds") || msg.contains("insufficient balance") {
        return (false, "insufficient_funds");
    }

    // Server errors (-32000 to -32099 range) are often transient
    if (-32099..=-32000).contains(&code) && !msg.contains("revert") {
        return (true, "server_error");
    }

    (false, "failed")
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type2_tx_starts_with_0x02() {
        let gas = GasParams {
            eip1559: true,
            base_fee: 30_000_000_000,
            max_priority_fee: 2_000_000_000,
            max_fee_per_gas: 62_000_000_000,
            gas_limit: 65_000,
        };
        let tx = EthereumAdapter::encode_type2_tx(1, 5, &gas, &[0xAA; 20], &[]);
        assert_eq!(tx[0], 0x02);
    }

    #[test]
    fn type2_second_byte_is_rlp_list() {
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 0,
        };
        let tx = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &[]);
        // After the 0x02 prefix, the RLP list prefix should be >= 0xc0
        assert!(tx[1] >= 0xc0);
    }

    #[test]
    fn legacy_tx_is_rlp_list() {
        let tx = EthereumAdapter::encode_legacy_tx(1, 0, 20_000_000_000, 65000, &[0xAA; 20], &[]);
        // Legacy tx starts directly with RLP list prefix
        assert!(tx[0] >= 0xc0);
    }

    #[test]
    fn type2_contains_to_address() {
        let to = [0xDD; 20];
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 21_000,
        };
        let tx = EthereumAdapter::encode_type2_tx(1, 0, &gas, &to, &[]);
        // The 20-byte address must appear somewhere in the encoded tx
        assert!(tx.windows(20).any(|w| w == to));
    }

    #[test]
    fn type2_contains_calldata() {
        let calldata = erc20_abi::encode_transfer(&[0xFF; 20], 999);
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 65_000,
        };
        let tx = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &calldata);
        // Transfer selector must appear in the encoded tx
        assert!(tx.windows(4).any(|w| w == erc20_abi::TRANSFER_SELECTOR));
    }

    #[test]
    fn legacy_contains_to_address() {
        let to = [0xEE; 20];
        let tx = EthereumAdapter::encode_legacy_tx(1, 0, 1, 21_000, &to, &[]);
        assert!(tx.windows(20).any(|w| w == to));
    }

    #[test]
    fn legacy_contains_eip155_chain_id_suffix() {
        let tx = EthereumAdapter::encode_legacy_tx(1, 0, 0, 21_000, &[0; 20], &[]);
        // EIP-155: last 3 RLP items are chainId=1, r=empty, s=empty → 0x01, 0x80, 0x80
        let data = &tx[1..]; // skip list prefix
        let tail = &data[data.len() - 3..];
        assert_eq!(tail, &[0x01, 0x80, 0x80]);
    }

    #[test]
    fn type2_with_calldata_larger_than_without() {
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 65_000,
        };
        let without = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &[]);
        let calldata = erc20_abi::encode_transfer(&[0; 20], 1000);
        let with = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &calldata);
        assert!(with.len() > without.len());
    }

    #[test]
    fn type2_different_chain_ids_produce_different_encoding() {
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 21_000,
        };
        let tx1 = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &[]);
        let tx137 = EthereumAdapter::encode_type2_tx(137, 0, &gas, &[0; 20], &[]);
        assert_ne!(tx1, tx137);
    }

    #[test]
    fn type2_different_nonces_produce_different_encoding() {
        let gas = GasParams {
            eip1559: true,
            base_fee: 0,
            max_priority_fee: 0,
            max_fee_per_gas: 0,
            gas_limit: 21_000,
        };
        let tx0 = EthereumAdapter::encode_type2_tx(1, 0, &gas, &[0; 20], &[]);
        let tx42 = EthereumAdapter::encode_type2_tx(1, 42, &gas, &[0; 20], &[]);
        assert_ne!(tx0, tx42);
    }

    #[test]
    fn max_fee_calculation() {
        // maxFeePerGas = baseFee * 2 + priorityFee
        let base: u64 = 30_000_000_000; // 30 gwei
        let priority: u64 = 2_000_000_000; // 2 gwei
        let max_fee = base * BASE_FEE_MULTIPLIER + priority;
        assert_eq!(max_fee, 62_000_000_000); // 62 gwei
    }

    #[test]
    fn priority_fee_clamped() {
        let low: u64 = 50_000_000;
        assert_eq!(low.clamp(MIN_PRIORITY_FEE, MAX_PRIORITY_FEE), MIN_PRIORITY_FEE);
        let high: u64 = 100_000_000_000;
        assert_eq!(high.clamp(MIN_PRIORITY_FEE, MAX_PRIORITY_FEE), MAX_PRIORITY_FEE);
        let normal: u64 = 2_000_000_000;
        assert_eq!(normal.clamp(MIN_PRIORITY_FEE, MAX_PRIORITY_FEE), normal);
    }

    // ── Error classification ─────────────────────────

    #[test]
    fn classify_nonce_too_low() {
        let (retriable, cat) = classify_send_error(-32000, "nonce too low");
        assert!(retriable);
        assert_eq!(cat, "nonce_retry");
    }

    #[test]
    fn classify_nonce_geth_variant() {
        let (retriable, cat) = classify_send_error(-32000, "nonce is too low for next tx");
        assert!(retriable);
        assert_eq!(cat, "nonce_retry");
    }

    #[test]
    fn classify_replacement_underpriced() {
        let (retriable, cat) = classify_send_error(-32000, "replacement transaction underpriced");
        assert!(retriable);
        assert_eq!(cat, "underpriced");
    }

    #[test]
    fn classify_gas_price_too_low() {
        let (retriable, cat) = classify_send_error(-32000, "gas price too low to replace");
        assert!(retriable);
        assert_eq!(cat, "underpriced");
    }

    #[test]
    fn classify_already_known() {
        let (retriable, cat) = classify_send_error(-32000, "already known");
        assert!(!retriable); // not an error, tx is in mempool
        assert_eq!(cat, "already_known");
    }

    #[test]
    fn classify_already_imported() {
        let (retriable, cat) = classify_send_error(-32000, "transaction already imported");
        assert!(!retriable);
        assert_eq!(cat, "already_known");
    }

    #[test]
    fn classify_pool_full() {
        let (retriable, cat) = classify_send_error(-32000, "txpool is full");
        assert!(retriable);
        assert_eq!(cat, "pool_full");
    }

    #[test]
    fn classify_insufficient_funds() {
        let (retriable, cat) = classify_send_error(-32000, "insufficient funds for gas * price + value");
        assert!(!retriable);
        assert_eq!(cat, "insufficient_funds");
    }

    #[test]
    fn classify_server_error_retriable() {
        let (retriable, cat) = classify_send_error(-32050, "internal error");
        assert!(retriable);
        assert_eq!(cat, "server_error");
    }

    #[test]
    fn classify_revert_not_retriable() {
        let (retriable, cat) = classify_send_error(-32000, "execution revert");
        assert!(!retriable);
        assert_eq!(cat, "failed");
    }

    #[test]
    fn classify_unknown_error() {
        let (retriable, cat) = classify_send_error(-1, "something unknown happened");
        assert!(!retriable);
        assert_eq!(cat, "failed");
    }
}
