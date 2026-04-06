use async_trait::async_trait;

use super::chain::*;
use super::solana_signing;
use super::solana_tx;

/// Solana adapter using Ed25519 signing and JSON-RPC to a Solana validator.
pub struct SolanaAdapter {
    client: reqwest::Client,
    endpoint: String,
}

impl SolanaAdapter {
    pub fn new(endpoint: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            endpoint: endpoint.to_string(),
        }
    }

    /// Call a Solana JSON-RPC method.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ChainError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        if let Some(err) = json.get("error") {
            return Err(ChainError::Rpc(err.to_string()));
        }

        json.get("result")
            .cloned()
            .ok_or_else(|| ChainError::Rpc("Missing result".into()))
    }
}

#[async_trait]
impl ChainAdapter for SolanaAdapter {
    fn chain_name(&self) -> &str {
        "solana"
    }

    fn required_confirmations(&self) -> u32 {
        31
    }

    async fn send_transaction(&self, tx: &SignedTx) -> Result<String, ChainError> {
        // Reconstruct SignedTransaction from raw bytes for retry support
        let encoded = solana_signing::bs58_encode(&tx.data);

        // Use retry wrapper for resilience
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [encoded, {
                "encoding": "base58",
                "skipPreflight": false,
                "preflightCommitment": "confirmed",
            }],
        });

        let mut last_err = String::new();
        for attempt in 0..3u32 {
            let resp = self.client.post(&self.endpoint).json(&body).send().await;
            match resp {
                Ok(r) => {
                    let json: serde_json::Value = r.json().await
                        .map_err(|e| ChainError::Rpc(e.to_string()))?;

                    if let Some(err) = json.get("error") {
                        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown");
                        if msg.contains("BlockhashNotFound") && attempt < 2 {
                            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
                            last_err = msg.to_string();
                            continue;
                        }
                        return Err(ChainError::Rpc(msg.to_string()));
                    }

                    return json.get("result")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string())
                        .ok_or_else(|| ChainError::Rpc("Missing result".into()));
                }
                Err(e) => {
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
                        last_err = e.to_string();
                        continue;
                    }
                    return Err(ChainError::Rpc(e.to_string()));
                }
            }
        }
        Err(ChainError::Rpc(format!("Max retries exceeded: {last_err}")))
    }

    async fn get_transaction_status(&self, tx_hash: &str) -> Result<TxState, ChainError> {
        let result = self
            .rpc_call(
                "getSignatureStatuses",
                serde_json::json!([[tx_hash], {"searchTransactionHistory": true}]),
            )
            .await?;

        let statuses = result
            .get("value")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ChainError::Rpc("Invalid status response".into()))?;

        let status = match statuses.first().and_then(|s| s.as_object()) {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(TxState::Pending),
        };

        if let Some(err) = status.get("err") {
            if !err.is_null() {
                return Ok(TxState::Failed {
                    reason: err.to_string(),
                });
            }
        }

        let slot = status
            .get("slot")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let confirmations = status
            .get("confirmations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let finalized = status
            .get("confirmationStatus")
            .and_then(|v| v.as_str())
            == Some("finalized");

        Ok(TxState::Confirmed {
            block: slot,
            confirmations: if finalized { 32 } else { confirmations },
        })
    }

    async fn estimate_fees(&self, _data: &SettlementData) -> Result<FeeEstimate, ChainError> {
        let base_fee: u64 = 5_000; // 5000 lamports per signature

        let result = self
            .rpc_call("getRecentPrioritizationFees", serde_json::json!([]))
            .await;

        let priority_fee = match result {
            Ok(val) => val
                .as_array()
                .and_then(|arr| {
                    let fees: Vec<u64> = arr
                        .iter()
                        .filter_map(|v| v.get("prioritizationFee")?.as_u64())
                        .collect();
                    if fees.is_empty() {
                        None
                    } else {
                        Some(fees.iter().sum::<u64>() / fees.len() as u64)
                    }
                })
                .unwrap_or(0),
            Err(_) => 0,
        };

        Ok(FeeEstimate {
            base_fee,
            priority_fee,
            total: base_fee + priority_fee,
            unit: "lamports".into(),
        })
    }

    async fn get_balance(&self, address: &str, token: &str) -> Result<u64, ChainError> {
        if token == "SOL" {
            let result = self
                .rpc_call("getBalance", serde_json::json!([address]))
                .await?;
            return result
                .get("value")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| ChainError::Rpc("Invalid balance response".into()));
        }

        // SPL token balance
        let result = self
            .rpc_call(
                "getTokenAccountsByOwner",
                serde_json::json!([
                    address,
                    {"mint": token},
                    {"encoding": "jsonParsed"}
                ]),
            )
            .await?;

        let accounts = result
            .get("value")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ChainError::Rpc("Invalid token balance response".into()))?;

        let balance = accounts
            .first()
            .and_then(|a| {
                a.get("account")?
                    .get("data")?
                    .get("parsed")?
                    .get("info")?
                    .get("tokenAmount")?
                    .get("amount")?
                    .as_str()
            })
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        Ok(balance)
    }

    async fn build_settlement_tx(
        &self,
        data: &SettlementData,
    ) -> Result<UnsignedTx, ChainError> {
        // Fetch recent blockhash
        let blockhash = solana_tx::fetch_recent_blockhash(&self.client, &self.endpoint)
            .await
            .map_err(|e| ChainError::Rpc(e))?;

        // Decode addresses from base58
        let from_bytes = decode_pubkey(&data.from)?;
        let to_bytes = decode_pubkey(&data.to)?;

        // Build SPL token transfer instruction
        let transfer_ix = solana_tx::spl_transfer_instruction(
            from_bytes,
            to_bytes,
            from_bytes, // authority = sender
            data.amount,
        );

        // Compose transaction message
        let msg = solana_tx::TransactionMessage::new(from_bytes, blockhash, vec![transfer_ix]);
        let serialised = msg.serialise();

        Ok(UnsignedTx {
            chain: "solana".into(),
            data: serialised,
        })
    }

    fn sign_transaction(
        &self,
        unsigned_tx: &UnsignedTx,
        private_key: &[u8; 32],
    ) -> Result<SignedTx, ChainError> {
        // Real Ed25519 signing via solana_signing module
        let sig = solana_signing::sign_transaction(private_key, &unsigned_tx.data)
            .map_err(ChainError::Signing)?;

        // Solana wire format: [signature_count(1)][signature(64)][message...]
        let mut tx_bytes = Vec::with_capacity(1 + 64 + unsigned_tx.data.len());
        tx_bytes.push(1); // 1 signature
        tx_bytes.extend_from_slice(&sig);
        tx_bytes.extend_from_slice(&unsigned_tx.data);

        Ok(SignedTx {
            chain: "solana".into(),
            data: tx_bytes,
        })
    }
}

/// Decode a base58 pubkey string into 32 bytes.
fn decode_pubkey(address: &str) -> Result<[u8; 32], ChainError> {
    let bytes = solana_signing::bs58_decode(address)
        .map_err(|e| ChainError::Signing(format!("Invalid address: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| ChainError::Signing("Address must be 32 bytes".into()))
}
