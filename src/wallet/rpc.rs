use serde::{Deserialize, Serialize};

/// JSON-RPC client for sending and querying blockchain transactions.
pub struct RpcClient {
    client: reqwest::Client,
    endpoint: String,
    chain_id: u64,
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
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            chain_id,
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

    /// Get current gas price.
    pub async fn gas_price(&self) -> Result<i64, RpcError> {
        let resp = self.call("eth_gasPrice", serde_json::json!([])).await?;
        let hex = resp
            .as_str()
            .ok_or_else(|| RpcError::Parse("Expected hex gas price".into()))?;
        i64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(e.to_string()))
    }

    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let body = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&body)
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
    }
}
