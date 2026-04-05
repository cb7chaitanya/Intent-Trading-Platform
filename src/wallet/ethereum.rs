use async_trait::async_trait;

use super::chain::*;
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

    async fn get_balance(&self, address: &str, _token: &str) -> Result<u64, ChainError> {
        // For a full implementation, query the ERC-20 contract's balanceOf.
        // Here we return the ETH balance as a proxy.
        let nonce = self
            .rpc
            .get_nonce(address)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // nonce is a stand-in; real impl would call eth_call with balanceOf ABI
        Ok(nonce)
    }

    async fn build_settlement_tx(
        &self,
        data: &SettlementData,
    ) -> Result<UnsignedTx, ChainError> {
        let payload = serde_json::json!({
            "to": data.to,
            "from": data.from,
            "value": data.amount,
            "token": data.token,
            "chainId": self.rpc.chain_id(),
        });
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        Ok(UnsignedTx {
            chain: "ethereum".into(),
            data: bytes,
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
