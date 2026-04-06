use serde::{Deserialize, Serialize};

use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitError};

/// JSON-RPC client for sending and querying blockchain transactions.
/// Wraps all calls with a circuit breaker to prevent cascading failures.
pub struct RpcClient {
    client: reqwest::Client,
    endpoint: String,
    chain_id: u64,
    breaker: CircuitBreaker,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: serde_json::Value,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReceipt {
    pub tx_hash: String,
    pub block_number: i64,
    pub gas_used: i64,
    pub status: bool, // true = success
}

#[derive(Debug)]
pub enum RpcError {
    Network(String),
    JsonRpc { code: i64, message: String },
    Parse(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Network(e) => write!(f, "RPC network error: {e}"),
            RpcError::JsonRpc { code, message } => {
                write!(f, "RPC error {code}: {message}")
            }
            RpcError::Parse(e) => write!(f, "RPC parse error: {e}"),
        }
    }
}

impl RpcClient {
    pub fn new(endpoint: &str, chain_id: u64) -> Self {
        let breaker_name = if chain_id == 1 || chain_id == 11155111 {
            "ethereum_rpc"
        } else {
            "evm_rpc"
        };
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            chain_id,
            breaker: CircuitBreaker::new(CircuitBreakerConfig::new(breaker_name, 5, 30)),
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Send a raw signed transaction to the network.
    /// Returns the transaction hash.
    pub async fn send_raw_transaction(&self, signed_tx_hex: &str) -> Result<String, RpcError> {
        let resp = self
            .call("eth_sendRawTransaction", serde_json::json!([signed_tx_hex]))
            .await?;
        resp.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| RpcError::Parse("Expected tx hash string".into()))
    }

    /// Get transaction receipt (returns None if not yet mined).
    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<Option<TxReceipt>, RpcError> {
        let resp = self
            .call(
                "eth_getTransactionReceipt",
                serde_json::json!([tx_hash]),
            )
            .await?;

        if resp.is_null() {
            return Ok(None);
        }

        let block_hex = resp["blockNumber"]
            .as_str()
            .unwrap_or("0x0");
        let gas_hex = resp["gasUsed"]
            .as_str()
            .unwrap_or("0x0");
        let status_hex = resp["status"]
            .as_str()
            .unwrap_or("0x0");

        Ok(Some(TxReceipt {
            tx_hash: tx_hash.to_string(),
            block_number: i64::from_str_radix(block_hex.trim_start_matches("0x"), 16)
                .unwrap_or(0),
            gas_used: i64::from_str_radix(gas_hex.trim_start_matches("0x"), 16).unwrap_or(0),
            status: status_hex == "0x1",
        }))
    }

    /// Get latest block number.
    pub async fn get_block_number(&self) -> Result<i64, RpcError> {
        let resp = self.call("eth_blockNumber", serde_json::json!([])).await?;
        let hex = resp
            .as_str()
            .ok_or_else(|| RpcError::Parse("Expected hex block number".into()))?;
        i64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Get nonce for an address.
    pub async fn get_nonce(&self, address: &str) -> Result<u64, RpcError> {
        let resp = self
            .call(
                "eth_getTransactionCount",
                serde_json::json!([address, "latest"]),
            )
            .await?;
        let hex = resp
            .as_str()
            .ok_or_else(|| RpcError::Parse("Expected hex nonce".into()))?;
        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Execute a read-only contract call (eth_call).
    pub async fn eth_call(&self, to: &str, data: &str) -> Result<String, RpcError> {
        let resp = self
            .call(
                "eth_call",
                serde_json::json!([{"to": to, "data": data}, "latest"]),
            )
            .await?;
        resp.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| RpcError::Parse("Expected hex string from eth_call".into()))
    }

    /// Get current gas price (legacy).
    pub async fn gas_price(&self) -> Result<i64, RpcError> {
        let resp = self.call("eth_gasPrice", serde_json::json!([])).await?;
        let hex = resp
            .as_str()
            .ok_or_else(|| RpcError::Parse("Expected hex gas price".into()))?;
        i64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Fetch the latest block's baseFeePerGas. Returns None for pre-EIP-1559 chains.
    pub async fn get_base_fee(&self) -> Result<Option<u64>, RpcError> {
        let resp = self
            .call("eth_getBlockByNumber", serde_json::json!(["latest", false]))
            .await?;

        let hex = match resp.get("baseFeePerGas").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return Ok(None), // pre-1559 block
        };
        let val = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))?;
        Ok(Some(val))
    }

    /// Fetch priority fee suggestion via eth_maxPriorityFeePerGas.
    /// Returns None if the node doesn't support this method.
    pub async fn max_priority_fee(&self) -> Result<Option<u64>, RpcError> {
        match self.call("eth_maxPriorityFeePerGas", serde_json::json!([])).await {
            Ok(resp) => {
                let hex = resp
                    .as_str()
                    .ok_or_else(|| RpcError::Parse("Expected hex priority fee".into()))?;
                let val = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                    .map_err(|e| RpcError::Parse(e.to_string()))?;
                Ok(Some(val))
            }
            Err(RpcError::JsonRpc { code: -32601, .. }) => Ok(None), // method not found
            Err(RpcError::JsonRpc { code: -32602, .. }) => Ok(None), // unsupported
            Err(e) => Err(e),
        }
    }

    /// Estimate gas for a transaction via eth_estimateGas.
    pub async fn estimate_gas(
        &self,
        from: &str,
        to: &str,
        data: &str,
    ) -> Result<u64, RpcError> {
        let resp = self
            .call(
                "eth_estimateGas",
                serde_json::json!([{
                    "from": from,
                    "to": to,
                    "data": data,
                }]),
            )
            .await?;
        let hex = resp
            .as_str()
            .ok_or_else(|| RpcError::Parse("Expected hex gas estimate".into()))?;
        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Get fee history for percentile-based priority fee estimation.
    /// Returns reward percentiles for the last `block_count` blocks.
    pub async fn fee_history(
        &self,
        block_count: u64,
        percentiles: &[f64],
    ) -> Result<Vec<u64>, RpcError> {
        let resp = self
            .call(
                "eth_feeHistory",
                serde_json::json!([
                    format!("0x{:x}", block_count),
                    "latest",
                    percentiles,
                ]),
            )
            .await?;

        // Extract reward array → flatten → take the requested percentile column
        let rewards = resp
            .get("reward")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let mut tips = Vec::new();
        for block_rewards in &rewards {
            if let Some(arr) = block_rewards.as_array() {
                if let Some(hex) = arr.first().and_then(|v| v.as_str()) {
                    let val = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                        .unwrap_or(0);
                    tips.push(val);
                }
            }
        }
        Ok(tips)
    }

    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let body = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };
        let body_json = serde_json::to_value(&body).unwrap_or_default();

        let result = self.breaker.call(async {
            let resp = client
                .post(&endpoint)
                .json(&body_json)
                .send()
                .await
                .map_err(|e| RpcError::Network(e.to_string()))?;

            let rpc_resp: JsonRpcResponse = resp
                .json()
                .await
                .map_err(|e| RpcError::Parse(e.to_string()))?;

            if let Some(err) = rpc_resp.error {
                return Err(RpcError::JsonRpc {
                    code: err.code,
                    message: err.message,
                });
            }

            rpc_resp
                .result
                .ok_or_else(|| RpcError::Parse("Missing result field".into()))
        }).await;

        match result {
            Ok(v) => Ok(v),
            Err(CircuitError::Open { breaker, remaining_secs }) => {
                Err(RpcError::Network(format!(
                    "Circuit breaker '{breaker}' open (resets in {remaining_secs}s)"
                )))
            }
            Err(CircuitError::Inner(e)) => Err(e),
        }
    }
}
