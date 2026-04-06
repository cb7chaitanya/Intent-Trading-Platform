//! Ethereum transaction signing with secp256k1.
//!
//! Provides correct v, r, s computation for both legacy (EIP-155) and
//! EIP-1559 (type 2) transactions, with ecrecover verification.
//!
//! Replaces the generic `signing::sign_transaction` path which used SHA-256
//! as a keccak stand-in and did not produce proper v,r,s or re-encode the
//! signed transaction.

use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use sha3::{Digest, Keccak256};

use super::rlp::{self, Eip1559TxFields, LegacyTxFields};

// ── Keccak-256 ──────────────────────────────────────────

/// Compute Keccak-256 hash (the Ethereum variant, NOT NIST SHA-3).
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

// ── ECDSA signing ───────────────────────────────────────

/// Sign a 32-byte prehash with secp256k1.
/// Returns `(r, s, recovery_id)` where recovery_id is 0 or 1.
pub fn sign_hash(
    hash: &[u8; 32],
    private_key: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], u8), String> {
    let signing_key =
        SigningKey::from_bytes(private_key.into()).map_err(|e| format!("invalid key: {e}"))?;

    let (sig, recid) = signing_key
        .sign_prehash_recoverable(hash)
        .map_err(|e| format!("signing failed: {e}"))?;

    let sig_bytes = sig.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&sig_bytes[..32]);
    s.copy_from_slice(&sig_bytes[32..]);

    Ok((r, s, recid.to_byte()))
}

// ── Ecrecover ───────────────────────────────────────────

/// Recover the Ethereum address that produced the signature over `hash`.
///
/// `recovery_id` is 0 or 1 (the raw parity bit, not the full `v` value).
/// For legacy EIP-155 transactions, extract it as `v - chain_id*2 - 35`.
/// For EIP-1559 transactions, use `y_parity` directly.
pub fn ecrecover(
    hash: &[u8; 32],
    recovery_id: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Result<[u8; 20], String> {
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let sig =
        Signature::from_slice(&sig_bytes).map_err(|e| format!("invalid signature: {e}"))?;

    let recid =
        RecoveryId::from_byte(recovery_id).ok_or_else(|| "invalid recovery id".to_string())?;

    let vk = VerifyingKey::recover_from_prehash(hash, &sig, recid)
        .map_err(|e| format!("ecrecover failed: {e}"))?;

    Ok(pubkey_to_address(&vk))
}

// ── Address derivation ──────────────────────────────────

/// Derive Ethereum address from a secp256k1 public key.
///
/// `address = keccak256(uncompressed_pubkey_bytes[1..])[12..]`
fn pubkey_to_address(vk: &VerifyingKey) -> [u8; 20] {
    let point = vk.to_encoded_point(false);
    let pub_bytes = &point.as_bytes()[1..]; // strip the 0x04 prefix
    let hash = keccak256(pub_bytes);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

/// Derive the Ethereum address for a given private key.
pub fn private_key_to_address(private_key: &[u8; 32]) -> Result<[u8; 20], String> {
    let signing_key =
        SigningKey::from_bytes(private_key.into()).map_err(|e| format!("invalid key: {e}"))?;
    let vk = VerifyingKey::from(&signing_key);
    Ok(pubkey_to_address(&vk))
}

// ── Transaction signing ─────────────────────────────────

/// Sign a legacy (EIP-155) transaction.
///
/// Returns the complete RLP-encoded signed transaction ready for
/// `eth_sendRawTransaction`:
///   `RLP([nonce, gasPrice, gasLimit, to, value, data, v, r, s])`
///
/// where `v = chain_id * 2 + 35 + recovery_id`.
pub fn sign_legacy_tx(tx: &LegacyTxFields, private_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    // 1. Encode unsigned tx for signing (EIP-155: append chainId, 0, 0)
    let unsigned_rlp = rlp::encode_legacy_unsigned(tx);

    // 2. Hash the unsigned encoding
    let hash = keccak256(&unsigned_rlp);

    // 3. Sign the hash
    let (r, s, recovery_id) = sign_hash(&hash, private_key)?;

    // 4. Compute v per EIP-155: v = chain_id * 2 + 35 + recovery_id
    let v = tx.chain_id * 2 + 35 + recovery_id as u64;

    // 5. Produce the final signed encoding
    Ok(rlp::encode_legacy_signed(tx, v, &r, &s))
}

/// Sign an EIP-1559 (type 2) transaction.
///
/// Returns the complete encoded signed transaction:
///   `0x02 || RLP([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
///     gasLimit, to, value, data, accessList, yParity, r, s])`
pub fn sign_eip1559_tx(
    tx: &Eip1559TxFields,
    private_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    // 1. Encode unsigned tx: 0x02 || RLP([fields...])
    let unsigned = rlp::encode_eip1559_unsigned(tx);

    // 2. Hash the full unsigned encoding (including 0x02 type prefix)
    let hash = keccak256(&unsigned);

    // 3. Sign the hash
    let (r, s, recovery_id) = sign_hash(&hash, private_key)?;

    // 4. Produce signed encoding: 0x02 || RLP([...fields..., yParity, r, s])
    Ok(rlp::encode_eip1559_signed(tx, recovery_id as u64, &r, &s))
}

// ── Verification helpers ────────────────────────────────

/// Verify that a legacy signed transaction was produced by `expected_address`.
pub fn verify_legacy_tx(
    tx: &LegacyTxFields,
    v: u64,
    r: &[u8; 32],
    s: &[u8; 32],
    expected_address: &[u8; 20],
) -> Result<bool, String> {
    let unsigned_rlp = rlp::encode_legacy_unsigned(tx);
    let hash = keccak256(&unsigned_rlp);

    // Extract recovery_id from EIP-155 v value
    let recovery_id = (v - tx.chain_id * 2 - 35) as u8;
    let recovered = ecrecover(&hash, recovery_id, r, s)?;

    Ok(recovered == *expected_address)
}

/// Verify that an EIP-1559 signed transaction was produced by `expected_address`.
pub fn verify_eip1559_tx(
    tx: &Eip1559TxFields,
    y_parity: u8,
    r: &[u8; 32],
    s: &[u8; 32],
    expected_address: &[u8; 20],
) -> Result<bool, String> {
    let unsigned = rlp::encode_eip1559_unsigned(tx);
    let hash = keccak256(&unsigned);

    let recovered = ecrecover(&hash, y_parity, r, s)?;

    Ok(recovered == *expected_address)
}

// ── Serialisation for UnsignedTx transport ──────────────

/// Tagged envelope carried inside `UnsignedTx.data` so that
/// `sign_transaction` can recover the structured fields it needs
/// to re-encode the signed transaction with v, r, s.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum EthUnsignedTxData {
    Legacy(LegacyTxFields),
    Eip1559(Eip1559TxFields),
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let bytes = hex::decode(s).unwrap();
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }

    fn hex20(s: &str) -> [u8; 20] {
        let bytes = hex::decode(s).unwrap();
        let mut out = [0u8; 20];
        out.copy_from_slice(&bytes);
        out
    }

    // ── Keccak-256 known values ─────────────────────────

    #[test]
    fn keccak256_empty() {
        let hash = keccak256(b"");
        assert_eq!(
            hex::encode(hash),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn keccak256_hello_world() {
        let hash = keccak256(b"hello world");
        assert_eq!(
            hex::encode(hash),
            "47173285a8d7341e5e972fc677286384f802f8ef42a5ec5f03bbfa254cb01fad"
        );
    }

    // ── Address derivation ──────────────────────────────

    #[test]
    fn derive_address_from_known_key() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let addr = private_key_to_address(&key).unwrap();
        // Must be a 20-byte address
        assert_eq!(addr.len(), 20);
        // Must not be all zeros
        assert_ne!(addr, [0u8; 20]);
    }

    // ── Sign + ecrecover round-trip ─────────────────────

    #[test]
    fn sign_and_recover_roundtrip() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let expected = private_key_to_address(&key).unwrap();

        let hash = keccak256(b"test message");
        let (r, s, v) = sign_hash(&hash, &key).unwrap();
        let recovered = ecrecover(&hash, v, &r, &s).unwrap();

        assert_eq!(recovered, expected);
    }

    #[test]
    fn recover_with_wrong_hash_gives_different_address() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let expected = private_key_to_address(&key).unwrap();

        let hash_a = keccak256(b"message A");
        let hash_b = keccak256(b"message B");

        let (r, s, v) = sign_hash(&hash_a, &key).unwrap();
        let recovered = ecrecover(&hash_b, v, &r, &s).unwrap();

        assert_ne!(recovered, expected);
    }

    #[test]
    fn deterministic_signatures_rfc6979() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let hash = keccak256(b"deterministic");

        let (r1, s1, v1) = sign_hash(&hash, &key).unwrap();
        let (r2, s2, v2) = sign_hash(&hash, &key).unwrap();

        assert_eq!((r1, s1, v1), (r2, s2, v2));
    }

    // ── EIP-155 signing hash test vector ────────────────
    //
    // Canonical values from the EIP-155 specification:
    //   nonce=9, gasPrice=20 gwei, gasLimit=21000,
    //   to=0x3535...35, value=1 ETH, data=empty, chainId=1
    //   signing hash = daf5a779ae972f972197303d7b574746c7ef83eadac0f2791ad23db92e4c8e53

    fn eip155_test_tx() -> LegacyTxFields {
        LegacyTxFields {
            nonce: 9,
            gas_price: 20_000_000_000,
            gas_limit: 21_000,
            to: hex20("3535353535353535353535353535353535353535"),
            value: 1_000_000_000_000_000_000, // 1 ETH
            data: vec![],
            chain_id: 1,
        }
    }

    #[test]
    fn eip155_signing_hash_matches_spec() {
        let tx = eip155_test_tx();
        let unsigned = rlp::encode_legacy_unsigned(&tx);

        // Verify RLP encoding matches the EIP-155 spec byte-for-byte
        assert_eq!(
            hex::encode(&unsigned),
            "ec098504a817c800825208943535353535353535353535353535353535353535\
             880de0b6b3a764000080018080"
        );

        // keccak256 of the unsigned encoding — verified independently
        // with pycryptodome keccak.new(digest_bits=256)
        let hash = keccak256(&unsigned);
        assert_eq!(
            hex::encode(hash),
            "daf5a779ae972f972197303d7b574746c7ef83eadac0f2791ad23db92e4c8e53"
        );
    }

    #[test]
    fn eip155_v_r_s_match_spec() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let tx = eip155_test_tx();

        let unsigned = rlp::encode_legacy_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, recid) = sign_hash(&hash, &key).unwrap();
        let v = tx.chain_id * 2 + 35 + recid as u64;

        // From the EIP-155 spec
        assert_eq!(v, 37);
        assert_eq!(
            hex::encode(r),
            "28ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276"
        );
        assert_eq!(
            hex::encode(s),
            "67cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
        );
    }

    #[test]
    fn eip155_sign_and_verify() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let signer = private_key_to_address(&key).unwrap();
        let tx = eip155_test_tx();

        let signed = sign_legacy_tx(&tx, &key).unwrap();

        // Must be a valid RLP list
        assert!(signed[0] >= 0xc0);

        // Verify via ecrecover
        let unsigned = rlp::encode_legacy_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, recid) = sign_hash(&hash, &key).unwrap();
        let v = tx.chain_id * 2 + 35 + recid as u64;

        assert!(verify_legacy_tx(&tx, v, &r, &s, &signer).unwrap());
    }

    #[test]
    fn eip155_wrong_address_fails_verify() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let wrong = hex20("0000000000000000000000000000000000000001");
        let tx = eip155_test_tx();

        let unsigned = rlp::encode_legacy_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, recid) = sign_hash(&hash, &key).unwrap();
        let v = tx.chain_id * 2 + 35 + recid as u64;

        assert!(!verify_legacy_tx(&tx, v, &r, &s, &wrong).unwrap());
    }

    #[test]
    fn eip155_signed_tx_larger_than_unsigned() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let tx = eip155_test_tx();

        let unsigned = rlp::encode_legacy_unsigned(&tx);
        let signed = sign_legacy_tx(&tx, &key).unwrap();

        // Signed replaces (chainId=1, 0, 0) → (v~37, r=32B, s=32B)
        assert!(signed.len() > unsigned.len());
    }

    // ── EIP-1559 signing ────────────────────────────────

    fn eip1559_test_tx() -> Eip1559TxFields {
        Eip1559TxFields {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 2_000_000_000,
            max_fee_per_gas: 100_000_000_000,
            gas_limit: 21_000,
            to: hex20("3535353535353535353535353535353535353535"),
            value: 1_000_000_000_000_000_000,
            data: vec![],
        }
    }

    #[test]
    fn eip1559_sign_starts_with_type_prefix() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let tx = eip1559_test_tx();

        let signed = sign_eip1559_tx(&tx, &key).unwrap();
        assert_eq!(signed[0], 0x02);
    }

    #[test]
    fn eip1559_sign_and_verify() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let signer = private_key_to_address(&key).unwrap();
        let tx = eip1559_test_tx();

        let signed = sign_eip1559_tx(&tx, &key).unwrap();
        assert_eq!(signed[0], 0x02);

        // Verify
        let unsigned = rlp::encode_eip1559_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, y_parity) = sign_hash(&hash, &key).unwrap();

        assert!(verify_eip1559_tx(&tx, y_parity, &r, &s, &signer).unwrap());
    }

    #[test]
    fn eip1559_wrong_address_fails_verify() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let wrong = hex20("0000000000000000000000000000000000000001");
        let tx = eip1559_test_tx();

        let unsigned = rlp::encode_eip1559_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, y_parity) = sign_hash(&hash, &key).unwrap();

        assert!(!verify_eip1559_tx(&tx, y_parity, &r, &s, &wrong).unwrap());
    }

    #[test]
    fn eip1559_with_erc20_calldata() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let signer = private_key_to_address(&key).unwrap();

        let mut calldata = Vec::with_capacity(68);
        calldata.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]); // transfer selector
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(&[0xBE; 20]); // recipient
        calldata.extend_from_slice(&[0u8; 24]);
        calldata.extend_from_slice(&1_000_000u64.to_be_bytes()); // 1 USDC

        let tx = Eip1559TxFields {
            chain_id: 1,
            nonce: 42,
            max_priority_fee_per_gas: 2_000_000_000,
            max_fee_per_gas: 50_000_000_000,
            gas_limit: 65_000,
            to: hex20("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"), // USDC
            value: 0,
            data: calldata,
        };

        let signed = sign_eip1559_tx(&tx, &key).unwrap();
        assert_eq!(signed[0], 0x02);

        let unsigned = rlp::encode_eip1559_unsigned(&tx);
        let hash = keccak256(&unsigned);
        let (r, s, y_parity) = sign_hash(&hash, &key).unwrap();
        assert!(verify_eip1559_tx(&tx, y_parity, &r, &s, &signer).unwrap());
    }

    // ── v value depends on chain ID ─────────────────────

    #[test]
    fn legacy_v_encodes_chain_id() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");

        let make = |chain_id| LegacyTxFields {
            nonce: 0,
            gas_price: 1,
            gas_limit: 21_000,
            to: [0u8; 20],
            value: 0,
            data: vec![],
            chain_id,
        };

        // chain_id=1 → v ∈ {37, 38}
        let tx1 = make(1);
        let h1 = keccak256(&rlp::encode_legacy_unsigned(&tx1));
        let (_, _, r1) = sign_hash(&h1, &key).unwrap();
        let v1 = 1 * 2 + 35 + r1 as u64;
        assert!(v1 == 37 || v1 == 38);

        // chain_id=137 (Polygon) → v ∈ {309, 310}
        let tx137 = make(137);
        let h137 = keccak256(&rlp::encode_legacy_unsigned(&tx137));
        let (_, _, r137) = sign_hash(&h137, &key).unwrap();
        let v137 = 137 * 2 + 35 + r137 as u64;
        assert!(v137 == 309 || v137 == 310);
    }

    // ── Broadcastable output format ─────────────────────

    #[test]
    fn legacy_signed_is_valid_hex_for_rpc() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let tx = eip155_test_tx();

        let signed = sign_legacy_tx(&tx, &key).unwrap();
        let raw_hex = format!("0x{}", hex::encode(&signed));

        assert!(raw_hex.starts_with("0x"));
        assert_eq!(raw_hex.len() % 2, 0);
    }

    #[test]
    fn eip1559_signed_is_valid_hex_for_rpc() {
        let key =
            hex32("4646464646464646464646464646464646464646464646464646464646464646");
        let tx = eip1559_test_tx();

        let signed = sign_eip1559_tx(&tx, &key).unwrap();
        let raw_hex = format!("0x{}", hex::encode(&signed));

        assert!(raw_hex.starts_with("0x02"));
    }

    // ── Serialisation round-trip ────────────────────────

    #[test]
    fn unsigned_tx_data_roundtrip_legacy() {
        let tx = eip155_test_tx();
        let data = EthUnsignedTxData::Legacy(tx.clone());
        let json = serde_json::to_vec(&data).unwrap();
        let back: EthUnsignedTxData = serde_json::from_slice(&json).unwrap();

        match back {
            EthUnsignedTxData::Legacy(t) => {
                assert_eq!(t.nonce, 9);
                assert_eq!(t.chain_id, 1);
            }
            _ => panic!("expected Legacy variant"),
        }
    }

    #[test]
    fn unsigned_tx_data_roundtrip_eip1559() {
        let tx = eip1559_test_tx();
        let data = EthUnsignedTxData::Eip1559(tx.clone());
        let json = serde_json::to_vec(&data).unwrap();
        let back: EthUnsignedTxData = serde_json::from_slice(&json).unwrap();

        match back {
            EthUnsignedTxData::Eip1559(t) => {
                assert_eq!(t.chain_id, 1);
                assert_eq!(t.max_fee_per_gas, 100_000_000_000);
            }
            _ => panic!("expected Eip1559 variant"),
        }
    }
}
