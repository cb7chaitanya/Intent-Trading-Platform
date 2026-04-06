use async_trait::async_trait;

use super::chain::*;
use super::erc20_abi;
use super::rpc::RpcClient;
use super::signing;

/// Ethereum adapter backed by the existing JSON-RPC client.
pub struct EthereumAdapter {
    rpc: RpcClient,
}

impl EthereumAdapter {
    pub fn new(endpoint: &str, chain_id: u64) -> Self {
        Self {
            rpc: RpcClient::new(endpoint, chain_id),
        }
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
        let hex = format!("0x{}", hex::encode(&tx.data));
        self.rpc
            .send_raw_transaction(&hex)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))
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

    async fn estimate_fees(&self, _data: &SettlementData) -> Result<FeeEstimate, ChainError> {
        let gas_price = self
            .rpc
            .gas_price()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))? as u64;

        // ERC-20 transfer gas estimate: ~65,000
        let gas_limit: u64 = 65_000;
        let total = gas_price * gas_limit;

        Ok(FeeEstimate {
            base_fee: gas_price,
            priority_fee: 0,
            total,
            unit: "wei".into(),
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

        // Result is a hex-encoded uint256. Parse the last 8 bytes as u64.
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

        // Build ERC-20 transfer calldata
        let calldata = erc20_abi::encode_transfer(&to_addr, data.amount as u128);

        // Fetch nonce for the sender
        let nonce = self.rpc.get_nonce(&data.from)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let gas_price = self.rpc.gas_price()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))? as u64;

        // Encode as an EIP-155 unsigned transaction envelope:
        // [nonce, gasPrice, gasLimit, to (token contract), value (0 for ERC-20), data, chainId, 0, 0]
        // We serialize as a compact binary format that sign_transaction expects.
        let token_contract = erc20_abi::parse_address(&data.token)
            .map_err(|e| ChainError::Rpc(format!("Bad token address: {e}")))?;

        let chain_id = self.rpc.chain_id();
        let gas_limit: u64 = 65_000;

        let mut tx_data = Vec::with_capacity(256);
        // Header: chain_id (8) + nonce (8) + gas_price (8) + gas_limit (8) + to (20) + value (8) + calldata
        tx_data.extend_from_slice(&chain_id.to_be_bytes());     // 8 bytes
        tx_data.extend_from_slice(&nonce.to_be_bytes());         // 8 bytes
        tx_data.extend_from_slice(&gas_price.to_be_bytes());     // 8 bytes
        tx_data.extend_from_slice(&gas_limit.to_be_bytes());     // 8 bytes
        tx_data.extend_from_slice(&token_contract);               // 20 bytes (to = ERC-20 contract)
        tx_data.extend_from_slice(&0u64.to_be_bytes());           // 8 bytes (value = 0 for ERC-20)
        tx_data.extend_from_slice(&(calldata.len() as u32).to_be_bytes()); // 4 bytes calldata length
        tx_data.extend_from_slice(&calldata);                     // 68 bytes (transfer calldata)

        Ok(UnsignedTx {
            chain: "ethereum".into(),
            data: tx_data,
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
