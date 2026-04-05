use async_trait::async_trait;

use super::chain::*;
use super::signing;

/// Solana adapter using JSON-RPC to a Solana validator node.
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
        // Solana finality: ~31 confirmations (roughly 12-15 seconds)
        31
    }

    async fn send_transaction(&self, tx: &SignedTx) -> Result<String, ChainError> {
        let encoded = bs58_encode(&tx.data);
        let result = self
            .rpc_call(
                "sendTransaction",
                serde_json::json!([encoded, {"encoding": "base58"}]),
            )
            .await?;

        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ChainError::Rpc("Expected tx signature string".into()))
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

        // Check for error
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

        // confirmationStatus: "finalized" means fully confirmed
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
        // Solana has a fixed base fee of 5000 lamports per signature,
        // plus priority fees set by the user.
        let base_fee: u64 = 5_000;

        // Get recent priority fee levels
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
            // Native SOL balance
            let result = self
                .rpc_call("getBalance", serde_json::json!([address]))
                .await?;
            return result
                .get("value")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| ChainError::Rpc("Invalid balance response".into()));
        }

        // SPL token balance — query getTokenAccountsByOwner
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
        // Build a Solana transaction instruction.
        // In production this would construct a proper Solana transaction
        // with recent blockhash, program IDs, and account keys.
        let recent_blockhash = self
            .rpc_call("getLatestBlockhash", serde_json::json!([]))
            .await
            .ok()
            .and_then(|v| v.get("value")?.get("blockhash")?.as_str().map(String::from))
            .unwrap_or_default();

        let payload = serde_json::json!({
            "from": data.from,
            "to": data.to,
            "amount": data.amount,
            "token": data.token,
            "recentBlockhash": recent_blockhash,
        });

        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        Ok(UnsignedTx {
            chain: "solana".into(),
            data: bytes,
        })
    }

    fn sign_transaction(
        &self,
        unsigned_tx: &UnsignedTx,
        private_key: &[u8; 32],
    ) -> Result<SignedTx, ChainError> {
        // Ed25519 signing for Solana.
        // In production, use ed25519-dalek. For now, use our ECDSA signer
        // as a placeholder — the trait boundary is what matters.
        let sig =
            signing::sign_data(private_key, &unsigned_tx.data).map_err(ChainError::Signing)?;

        // Solana transaction = signature + message
        let mut tx_bytes = Vec::with_capacity(sig.len() + unsigned_tx.data.len());
        tx_bytes.extend_from_slice(&sig);
        tx_bytes.extend_from_slice(&unsigned_tx.data);

        Ok(SignedTx {
            chain: "solana".into(),
            data: tx_bytes,
        })
    }
}

/// Base58 encode bytes (minimal implementation for Solana).
fn bs58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if data.is_empty() {
        return String::new();
    }

    // Count leading zeros
    let zeros = data.iter().take_while(|&&b| b == 0).count();

    let mut digits: Vec<u8> = Vec::new();
    for &byte in data {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }

    let mut result = String::with_capacity(zeros + digits.len());
    for _ in 0..zeros {
        result.push('1');
    }
    for d in digits.iter().rev() {
        result.push(ALPHABET[*d as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bs58_encode_empty() {
        assert_eq!(bs58_encode(&[]), "");
    }

    #[test]
    fn bs58_encode_known() {
        // "Hello" in base58
        assert_eq!(bs58_encode(b"Hello"), "9Ajdvzr");
    }

    #[test]
    fn bs58_encode_leading_zeros() {
        assert_eq!(&bs58_encode(&[0, 0, 1])[..2], "11");
    }
}
