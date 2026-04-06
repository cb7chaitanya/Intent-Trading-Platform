//! Solana transaction building, signing, and submission.
//!
//! Constructs raw Solana transactions without depending on solana-sdk,
//! using only ed25519-dalek for signing and our JSON-RPC client for
//! blockhash fetching and submission.

use serde::Serialize;

use super::solana_signing;

// ── Wire format constants ────────────────────────────────

/// SPL Token program ID (base58: TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)
const SPL_TOKEN_PROGRAM: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172,
    28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

// ── Instruction ──────────────────────────────────────────

/// A Solana instruction (program_id + accounts + data).
#[derive(Debug, Clone)]
pub struct Instruction {
    pub program_id: [u8; 32],
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AccountMeta {
    pub pubkey: [u8; 32],
    pub is_signer: bool,
    pub is_writable: bool,
}

// ── Transaction ──────────────────────────────────────────

/// A Solana transaction message (unsigned).
#[derive(Debug, Clone)]
pub struct TransactionMessage {
    pub recent_blockhash: [u8; 32],
    pub instructions: Vec<Instruction>,
    pub fee_payer: [u8; 32],
}

/// A signed transaction ready for submission.
#[derive(Debug, Clone)]
pub struct SignedTransaction {
    pub signatures: Vec<[u8; 64]>,
    pub message: Vec<u8>,
}

// ── Instruction Builders ─────────────────────────────────

/// Build an SPL Token transfer instruction.
pub fn spl_transfer_instruction(
    source: [u8; 32],
    destination: [u8; 32],
    authority: [u8; 32],
    amount: u64,
) -> Instruction {
    // SPL Token Transfer instruction index = 3
    // Data: [3u8] + [amount as le bytes (8)]
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());

    Instruction {
        program_id: SPL_TOKEN_PROGRAM,
        accounts: vec![
            AccountMeta { pubkey: source, is_signer: false, is_writable: true },
            AccountMeta { pubkey: destination, is_signer: false, is_writable: true },
            AccountMeta { pubkey: authority, is_signer: true, is_writable: false },
        ],
        data,
    }
}

/// Build a program invocation instruction for the IntentX settlement program.
/// Encodes the `settle` instruction with Anchor discriminator.
pub fn settle_instruction(
    program_id: [u8; 32],
    config: [u8; 32],
    authority: [u8; 32],
    mint: [u8; 32],
    buyer_account: [u8; 32],
    seller_account: [u8; 32],
    fee_account: [u8; 32],
    amount: u64,
    fill_id: [u8; 16],
) -> Instruction {
    // Anchor discriminator for "settle" = sha256("global:settle")[..8]
    let discriminator = anchor_discriminator("global:settle");
    let mut data = discriminator.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&fill_id);

    Instruction {
        program_id,
        accounts: vec![
            AccountMeta { pubkey: config, is_signer: false, is_writable: true },
            AccountMeta { pubkey: authority, is_signer: true, is_writable: false },
            AccountMeta { pubkey: mint, is_signer: false, is_writable: false },
            AccountMeta { pubkey: buyer_account, is_signer: false, is_writable: true },
            AccountMeta { pubkey: seller_account, is_signer: false, is_writable: true },
            AccountMeta { pubkey: fee_account, is_signer: false, is_writable: true },
        ],
        data,
    }
}

/// Compute the Anchor 8-byte discriminator from a namespace:name string.
fn anchor_discriminator(name: &str) -> [u8; 8] {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(name.as_bytes());
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

// ── Message Serialisation ────────────────────────────────

impl TransactionMessage {
    /// Compose a transaction from multiple instructions.
    pub fn new(fee_payer: [u8; 32], recent_blockhash: [u8; 32], instructions: Vec<Instruction>) -> Self {
        Self {
            fee_payer,
            recent_blockhash,
            instructions,
        }
    }

    /// Serialise the message into the Solana wire format.
    ///
    /// Format:
    /// - compact-u16: num_required_signatures
    /// - compact-u16: num_readonly_signed
    /// - compact-u16: num_readonly_unsigned
    /// - compact-u16: num_accounts
    /// - [pubkey; num_accounts]  (32 bytes each)
    /// - recent_blockhash (32 bytes)
    /// - compact-u16: num_instructions
    /// - for each instruction:
    ///     - u8: program_id_index
    ///     - compact-u16: num_accounts
    ///     - [u8; num_accounts] (account indexes)
    ///     - compact-u16: data_len
    ///     - [u8; data_len]
    pub fn serialise(&self) -> Vec<u8> {
        // Collect unique accounts in order: signers-writable, signers-readonly,
        // non-signers-writable, non-signers-readonly
        let mut accounts: Vec<(u8, [u8; 32])> = Vec::new(); // (flags, pubkey)
        let mut seen = std::collections::HashSet::new();

        // Fee payer is always first (signer + writable)
        accounts.push((0b11, self.fee_payer));
        seen.insert(self.fee_payer);

        for ix in &self.instructions {
            for acc in &ix.accounts {
                if !seen.contains(&acc.pubkey) {
                    let flags = ((acc.is_signer as u8) << 1) | (acc.is_writable as u8);
                    accounts.push((flags, acc.pubkey));
                    seen.insert(acc.pubkey);
                }
            }
            if !seen.contains(&ix.program_id) {
                accounts.push((0b00, ix.program_id));
                seen.insert(ix.program_id);
            }
        }

        // Sort: signers first, then by writable
        accounts.sort_by(|a, b| {
            let a_signer = (a.0 >> 1) & 1;
            let b_signer = (b.0 >> 1) & 1;
            b_signer.cmp(&a_signer).then_with(|| {
                let a_writable = a.0 & 1;
                let b_writable = b.0 & 1;
                b_writable.cmp(&a_writable)
            })
        });

        // Ensure fee payer is at index 0
        if let Some(pos) = accounts.iter().position(|a| a.1 == self.fee_payer) {
            if pos != 0 {
                let fp = accounts.remove(pos);
                accounts.insert(0, fp);
            }
        }

        let num_signers = accounts.iter().filter(|a| (a.0 >> 1) & 1 == 1).count();
        let num_readonly_signed = accounts.iter()
            .filter(|a| (a.0 >> 1) & 1 == 1 && a.0 & 1 == 0)
            .count();
        let num_readonly_unsigned = accounts.iter()
            .filter(|a| (a.0 >> 1) & 1 == 0 && a.0 & 1 == 0)
            .count();

        let account_keys: Vec<[u8; 32]> = accounts.iter().map(|a| a.1).collect();

        let index_of = |pubkey: &[u8; 32]| -> u8 {
            account_keys.iter().position(|k| k == pubkey).unwrap_or(0) as u8
        };

        let mut buf = Vec::new();

        // Header
        buf.push(num_signers as u8);
        buf.push(num_readonly_signed as u8);
        buf.push(num_readonly_unsigned as u8);

        // Account keys
        encode_compact_u16(&mut buf, account_keys.len() as u16);
        for key in &account_keys {
            buf.extend_from_slice(key);
        }

        // Recent blockhash
        buf.extend_from_slice(&self.recent_blockhash);

        // Instructions
        encode_compact_u16(&mut buf, self.instructions.len() as u16);
        for ix in &self.instructions {
            buf.push(index_of(&ix.program_id));

            encode_compact_u16(&mut buf, ix.accounts.len() as u16);
            for acc in &ix.accounts {
                buf.push(index_of(&acc.pubkey));
            }

            encode_compact_u16(&mut buf, ix.data.len() as u16);
            buf.extend_from_slice(&ix.data);
        }

        buf
    }
}

/// Compact-u16 encoding used in Solana wire format.
fn encode_compact_u16(buf: &mut Vec<u8>, val: u16) {
    if val < 0x80 {
        buf.push(val as u8);
    } else if val < 0x4000 {
        buf.push(((val & 0x7f) | 0x80) as u8);
        buf.push((val >> 7) as u8);
    } else {
        buf.push(((val & 0x7f) | 0x80) as u8);
        buf.push((((val >> 7) & 0x7f) | 0x80) as u8);
        buf.push((val >> 14) as u8);
    }
}

// ── Signing ──────────────────────────────────────────────

impl SignedTransaction {
    /// Sign a transaction message with one or more private keys.
    pub fn sign(message: &TransactionMessage, signers: &[[u8; 32]]) -> Result<Self, String> {
        let serialised = message.serialise();
        let mut signatures = Vec::with_capacity(signers.len());

        for seed in signers {
            let sig_bytes = solana_signing::sign(seed, &serialised)?;
            let sig: [u8; 64] = sig_bytes
                .try_into()
                .map_err(|_| "Signature not 64 bytes".to_string())?;
            signatures.push(sig);
        }

        Ok(Self {
            signatures,
            message: serialised,
        })
    }

    /// Encode as base58 for RPC submission.
    pub fn to_base58(&self) -> String {
        solana_signing::bs58_encode(&self.to_bytes())
    }

    /// Raw bytes: [sig_count][signatures][message]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_compact_u16(&mut buf, self.signatures.len() as u16);
        for sig in &self.signatures {
            buf.extend_from_slice(sig);
        }
        buf.extend_from_slice(&self.message);
        buf
    }
}

// ── RPC Helpers ──────────────────────────────────────────

/// Fetch recent blockhash from Solana RPC and decode from base58.
pub async fn fetch_recent_blockhash(client: &reqwest::Client, endpoint: &str) -> Result<[u8; 32], String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLatestBlockhash",
        "params": [{"commitment": "finalized"}],
    });

    let resp: serde_json::Value = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let hash_str = resp
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.get("blockhash"))
        .and_then(|b| b.as_str())
        .ok_or("Missing blockhash in response")?;

    let bytes = solana_signing::bs58_decode(hash_str)?;
    let hash: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "Blockhash not 32 bytes".to_string())?;
    Ok(hash)
}

/// Send a signed transaction via RPC with retry.
pub async fn send_transaction_with_retry(
    client: &reqwest::Client,
    endpoint: &str,
    signed_tx: &SignedTransaction,
    max_retries: u32,
) -> Result<String, String> {
    let encoded = signed_tx.to_base58();

    for attempt in 0..=max_retries {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [encoded, {
                "encoding": "base58",
                "skipPreflight": false,
                "preflightCommitment": "confirmed",
                "maxRetries": 0,
            }],
        });

        let resp = client
            .post(endpoint)
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) => {
                let json: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;

                if let Some(err) = json.get("error") {
                    let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown");

                    // Retriable errors
                    if msg.contains("BlockhashNotFound") || msg.contains("Node is behind") {
                        if attempt < max_retries {
                            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
                            continue;
                        }
                    }

                    return Err(format!("RPC error: {msg}"));
                }

                if let Some(sig) = json.get("result").and_then(|r| r.as_str()) {
                    return Ok(sig.to_string());
                }

                return Err("Missing result in response".into());
            }
            Err(e) => {
                if attempt < max_retries {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
                    continue;
                }
                return Err(e.to_string());
            }
        }
    }

    Err("Max retries exceeded".into())
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_small() {
        let mut buf = Vec::new();
        encode_compact_u16(&mut buf, 5);
        assert_eq!(buf, vec![5]);
    }

    #[test]
    fn compact_u16_medium() {
        let mut buf = Vec::new();
        encode_compact_u16(&mut buf, 128);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn compact_u16_large() {
        let mut buf = Vec::new();
        encode_compact_u16(&mut buf, 16384);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn spl_transfer_instruction_format() {
        let src = [1u8; 32];
        let dst = [2u8; 32];
        let auth = [3u8; 32];
        let ix = spl_transfer_instruction(src, dst, auth, 1000);

        assert_eq!(ix.program_id, SPL_TOKEN_PROGRAM);
        assert_eq!(ix.accounts.len(), 3);
        assert_eq!(ix.data[0], 3); // Transfer instruction index
        assert_eq!(u64::from_le_bytes(ix.data[1..9].try_into().unwrap()), 1000);
    }

    #[test]
    fn settle_instruction_format() {
        let program = [10u8; 32];
        let config = [11u8; 32];
        let auth = [12u8; 32];
        let mint = [13u8; 32];
        let buyer = [14u8; 32];
        let seller = [15u8; 32];
        let fee_acc = [16u8; 32];
        let fill_id = [0xABu8; 16];

        let ix = settle_instruction(program, config, auth, mint, buyer, seller, fee_acc, 5000, fill_id);

        assert_eq!(ix.program_id, program);
        assert_eq!(ix.accounts.len(), 6);
        // Data: 8 (discriminator) + 8 (amount) + 16 (fill_id) = 32
        assert_eq!(ix.data.len(), 32);
        // Amount at offset 8
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 5000);
        // Fill ID at offset 16
        assert_eq!(&ix.data[16..32], &fill_id);
    }

    #[test]
    fn anchor_discriminator_deterministic() {
        let d1 = anchor_discriminator("global:settle");
        let d2 = anchor_discriminator("global:settle");
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 8);
    }

    #[test]
    fn transaction_message_serialises() {
        let payer = [1u8; 32];
        let blockhash = [2u8; 32];

        let ix = spl_transfer_instruction([3u8; 32], [4u8; 32], payer, 100);
        let msg = TransactionMessage::new(payer, blockhash, vec![ix]);
        let serialised = msg.serialise();

        // Should contain: header(3) + compact_len + accounts*32 + blockhash(32) + instructions
        assert!(serialised.len() > 3 + 32 + 32);
        // First byte = num signers (at least 1 for fee payer)
        assert!(serialised[0] >= 1);
    }

    #[test]
    fn sign_transaction_produces_valid_output() {
        let (seed, _) = solana_signing::generate_keypair();
        let payer = solana_signing::public_key_bytes(&seed);
        let blockhash = [0u8; 32];

        let ix = spl_transfer_instruction([3u8; 32], [4u8; 32], payer, 100);
        let msg = TransactionMessage::new(payer, blockhash, vec![ix]);
        let signed = SignedTransaction::sign(&msg, &[seed]).unwrap();

        assert_eq!(signed.signatures.len(), 1);
        assert_eq!(signed.signatures[0].len(), 64);
        assert!(!signed.message.is_empty());
    }

    #[test]
    fn signed_tx_to_bytes_includes_all_parts() {
        let (seed, _) = solana_signing::generate_keypair();
        let payer = solana_signing::public_key_bytes(&seed);
        let blockhash = [0u8; 32];

        let ix = spl_transfer_instruction([3u8; 32], [4u8; 32], payer, 100);
        let msg = TransactionMessage::new(payer, blockhash, vec![ix]);
        let signed = SignedTransaction::sign(&msg, &[seed]).unwrap();

        let bytes = signed.to_bytes();
        // At minimum: compact_u16(1) + 64 byte sig + message bytes
        assert!(bytes.len() >= 1 + 64 + signed.message.len());
    }

    #[test]
    fn multi_instruction_transaction() {
        let payer = [1u8; 32];
        let blockhash = [2u8; 32];

        let ix1 = spl_transfer_instruction([3u8; 32], [4u8; 32], payer, 100);
        let ix2 = spl_transfer_instruction([5u8; 32], [6u8; 32], payer, 200);

        let msg = TransactionMessage::new(payer, blockhash, vec![ix1, ix2]);
        let serialised = msg.serialise();

        // Should be larger than single instruction
        let single_ix = spl_transfer_instruction([3u8; 32], [4u8; 32], payer, 100);
        let single_msg = TransactionMessage::new(payer, blockhash, vec![single_ix]);
        let single_serialised = single_msg.serialise();

        assert!(serialised.len() > single_serialised.len());
    }
}
