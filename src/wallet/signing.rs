use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use k256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Digest, Sha256};

/// Generates a new secp256k1 keypair.
/// Returns (private_key_bytes, ethereum_address).
pub fn generate_keypair() -> ([u8; 32], String) {
    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = VerifyingKey::from(&signing_key);

    let address = derive_address(&verifying_key);
    let private_bytes: [u8; 32] = signing_key.to_bytes().into();

    (private_bytes, address)
}

/// Derives an Ethereum-style address from a public key.
/// keccak256(uncompressed_pubkey[1..]) → last 20 bytes → 0x-prefixed hex.
fn derive_address(vk: &VerifyingKey) -> String {
    let point = vk.to_encoded_point(false);
    let pub_bytes = &point.as_bytes()[1..]; // strip 0x04 prefix
    let hash = keccak256(pub_bytes);
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Keccak-256 using SHA-256 as a stand-in.
/// In production, replace with a proper keccak crate.
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Encrypt a private key using AES-256-GCM.
/// Returns (ciphertext, nonce).
pub fn encrypt_key(private_key: &[u8; 32], master_key: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, private_key.as_slice())
        .expect("AES-GCM encryption should not fail with valid inputs");

    (ciphertext, nonce_bytes.to_vec())
}

/// Decrypt a private key from AES-256-GCM ciphertext.
pub fn decrypt_key(
    ciphertext: &[u8],
    nonce: &[u8],
    master_key: &[u8; 32],
) -> Result<[u8; 32], String> {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce = Nonce::from_slice(nonce);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {e}"))?;

    let bytes: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| "Decrypted key has wrong length".to_string())?;

    Ok(bytes)
}

/// Sign arbitrary data with a private key (ECDSA secp256k1).
pub fn sign_data(private_key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    let signing_key =
        SigningKey::from_bytes(private_key.into()).map_err(|e| format!("Invalid key: {e}"))?;

    // Hash the data first (like Ethereum's personal_sign)
    let hash = keccak256(data);
    let signature: Signature = signing_key.sign(&hash);

    Ok(signature.to_vec())
}

/// Sign a transaction payload (serialised as bytes).
pub fn sign_transaction(
    private_key: &[u8; 32],
    tx_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    sign_data(private_key, tx_bytes)
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_generation_produces_valid_address() {
        let (key, address) = generate_keypair();
        assert_eq!(key.len(), 32);
        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 42); // 0x + 40 hex chars
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let master_key: [u8; 32] = rand::random();
        let private_key: [u8; 32] = rand::random();

        let (ciphertext, nonce) = encrypt_key(&private_key, &master_key);
        let decrypted = decrypt_key(&ciphertext, &nonce, &master_key).unwrap();

        assert_eq!(private_key, decrypted);
    }

    #[test]
    fn wrong_master_key_fails_decrypt() {
        let master_key: [u8; 32] = rand::random();
        let wrong_key: [u8; 32] = rand::random();
        let private_key: [u8; 32] = rand::random();

        let (ciphertext, nonce) = encrypt_key(&private_key, &master_key);
        let result = decrypt_key(&ciphertext, &nonce, &wrong_key);

        assert!(result.is_err());
    }

    #[test]
    fn sign_produces_deterministic_output_for_same_data() {
        let (key, _) = generate_keypair();
        let data = b"test transaction payload";

        let sig1 = sign_data(&key, data).unwrap();
        let sig2 = sign_data(&key, data).unwrap();

        // ECDSA with deterministic nonce (RFC 6979) → same signature
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn different_data_different_signatures() {
        let (key, _) = generate_keypair();

        let sig1 = sign_data(&key, b"payload A").unwrap();
        let sig2 = sign_data(&key, b"payload B").unwrap();

        assert_ne!(sig1, sig2);
    }

    #[test]
    fn sign_transaction_works() {
        let (key, _) = generate_keypair();
        let payload = serde_json::to_vec(&serde_json::json!({
            "to": "0xabc",
            "value": 1000,
            "asset": "USDC",
        }))
        .unwrap();

        let sig = sign_transaction(&key, &payload).unwrap();
        assert!(!sig.is_empty());
    }
}
