//! Ed25519 keypair generation, signing, and base58 encoding for Solana wallets.
//!
//! Solana uses Ed25519 (not secp256k1). The public key IS the on-chain address,
//! encoded as base58. Private keys are 64 bytes (32-byte seed + 32-byte public key)
//! but we store only the 32-byte seed and derive the full keypair on demand.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey, SECRET_KEY_LENGTH};

// ── Base58 ───────────────────────────────────────────────

const B58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Encode bytes to base58 (Bitcoin/Solana alphabet).
pub fn bs58_encode(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

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
        result.push(B58_ALPHABET[*d as usize] as char);
    }
    result
}

/// Decode base58 string to bytes.
pub fn bs58_decode(input: &str) -> Result<Vec<u8>, String> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let zeros = input.bytes().take_while(|&b| b == b'1').count();

    let mut bytes: Vec<u8> = Vec::new();
    for ch in input.bytes() {
        let val = B58_ALPHABET
            .iter()
            .position(|&c| c == ch)
            .ok_or_else(|| format!("Invalid base58 character: {}", ch as char))?
            as u32;

        let mut carry = val;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }

    let mut result = vec![0u8; zeros];
    result.extend(bytes.into_iter().rev());
    Ok(result)
}

// ── Keypair Generation ───────────────────────────────────

/// Generate a new Ed25519 keypair for Solana.
/// Returns (seed_bytes_32, base58_public_key_address).
pub fn generate_keypair() -> ([u8; 32], String) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key();

    let seed: [u8; SECRET_KEY_LENGTH] = signing_key.to_bytes();
    let address = get_public_key_address(&public_key);

    (seed, address)
}

/// Derive the base58 address from a 32-byte seed.
pub fn address_from_seed(seed: &[u8; 32]) -> Result<String, String> {
    let signing_key =
        SigningKey::from_bytes(seed);
    let public_key = signing_key.verifying_key();
    Ok(get_public_key_address(&public_key))
}

/// Get the base58-encoded public key (= Solana address).
pub fn get_public_key_address(vk: &VerifyingKey) -> String {
    bs58_encode(vk.as_bytes())
}

/// Get the raw 32-byte public key from a seed.
pub fn public_key_bytes(seed: &[u8; 32]) -> [u8; 32] {
    let signing_key = SigningKey::from_bytes(seed);
    signing_key.verifying_key().to_bytes()
}

// ── Signing ──────────────────────────────────────────────

/// Sign a message with Ed25519 using the 32-byte seed.
/// Returns the 64-byte signature.
pub fn sign(seed: &[u8; 32], message: &[u8]) -> Result<Vec<u8>, String> {
    let signing_key = SigningKey::from_bytes(seed);
    let signature = signing_key.sign(message);
    Ok(signature.to_vec())
}

/// Verify an Ed25519 signature against a public key and message.
pub fn verify(public_key: &[u8; 32], message: &[u8], signature: &[u8]) -> Result<bool, String> {
    let vk = VerifyingKey::from_bytes(public_key)
        .map_err(|e| format!("Invalid public key: {e}"))?;

    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| "Signature must be 64 bytes".to_string())?;

    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    Ok(vk.verify_strict(message, &sig).is_ok())
}

/// Sign a Solana transaction message.
/// Returns the 64-byte Ed25519 signature.
pub fn sign_transaction(seed: &[u8; 32], tx_message: &[u8]) -> Result<Vec<u8>, String> {
    sign(seed, tx_message)
}

// ── Key Encryption (delegates to same AES-256-GCM) ───────

/// Encrypt a 32-byte Ed25519 seed using AES-256-GCM.
pub fn encrypt_seed(seed: &[u8; 32], master_key: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, seed.as_slice())
        .expect("AES-GCM encryption should not fail with valid inputs");

    (ciphertext, nonce_bytes.to_vec())
}

/// Decrypt a 32-byte Ed25519 seed from AES-256-GCM ciphertext.
pub fn decrypt_seed(
    ciphertext: &[u8],
    nonce: &[u8],
    master_key: &[u8; 32],
) -> Result<[u8; 32], String> {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce = Nonce::from_slice(nonce);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {e}"))?;

    plaintext
        .try_into()
        .map_err(|_| "Decrypted seed has wrong length".to_string())
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_produces_valid_base58_address() {
        let (seed, address) = generate_keypair();
        assert_eq!(seed.len(), 32);
        // Solana addresses are 32-44 chars in base58
        assert!(address.len() >= 32 && address.len() <= 44, "address len: {}", address.len());
        // Should only contain base58 characters
        for ch in address.chars() {
            assert!(
                B58_ALPHABET.contains(&(ch as u8)),
                "Invalid char in address: {ch}"
            );
        }
    }

    #[test]
    fn address_from_seed_matches_generation() {
        let (seed, address) = generate_keypair();
        let derived = address_from_seed(&seed).unwrap();
        assert_eq!(address, derived);
    }

    #[test]
    fn public_key_roundtrip() {
        let (seed, address) = generate_keypair();
        let pubkey = public_key_bytes(&seed);
        let encoded = bs58_encode(&pubkey);
        assert_eq!(address, encoded);
    }

    #[test]
    fn sign_and_verify() {
        let (seed, _) = generate_keypair();
        let message = b"Hello Solana";

        let sig = sign(&seed, message).unwrap();
        assert_eq!(sig.len(), 64); // Ed25519 signatures are always 64 bytes

        let pubkey = public_key_bytes(&seed);
        assert!(verify(&pubkey, message, &sig).unwrap());
    }

    #[test]
    fn wrong_message_fails_verification() {
        let (seed, _) = generate_keypair();
        let sig = sign(&seed, b"correct message").unwrap();

        let pubkey = public_key_bytes(&seed);
        assert!(!verify(&pubkey, b"wrong message", &sig).unwrap());
    }

    #[test]
    fn wrong_key_fails_verification() {
        let (seed1, _) = generate_keypair();
        let (seed2, _) = generate_keypair();
        let message = b"test";

        let sig = sign(&seed1, message).unwrap();
        let wrong_pubkey = public_key_bytes(&seed2);
        assert!(!verify(&wrong_pubkey, message, &sig).unwrap());
    }

    #[test]
    fn deterministic_signatures() {
        let (seed, _) = generate_keypair();
        let msg = b"deterministic test";
        let sig1 = sign(&seed, msg).unwrap();
        let sig2 = sign(&seed, msg).unwrap();
        assert_eq!(sig1, sig2); // Ed25519 is deterministic
    }

    #[test]
    fn encrypt_decrypt_seed_roundtrip() {
        let master_key: [u8; 32] = rand::random();
        let (seed, _) = generate_keypair();

        let (ciphertext, nonce) = encrypt_seed(&seed, &master_key);
        let decrypted = decrypt_seed(&ciphertext, &nonce, &master_key).unwrap();

        assert_eq!(seed, decrypted);
    }

    #[test]
    fn wrong_master_key_fails_seed_decrypt() {
        let master_key: [u8; 32] = rand::random();
        let wrong_key: [u8; 32] = rand::random();
        let seed: [u8; 32] = rand::random();

        let (ciphertext, nonce) = encrypt_seed(&seed, &master_key);
        assert!(decrypt_seed(&ciphertext, &nonce, &wrong_key).is_err());
    }

    #[test]
    fn bs58_encode_decode_roundtrip() {
        let data = b"Hello Solana world";
        let encoded = bs58_encode(data);
        let decoded = bs58_decode(&encoded).unwrap();
        assert_eq!(data.to_vec(), decoded);
    }

    #[test]
    fn bs58_empty() {
        assert_eq!(bs58_encode(&[]), "");
        assert_eq!(bs58_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn bs58_leading_zeros() {
        let data = vec![0, 0, 0, 1, 2, 3];
        let encoded = bs58_encode(&data);
        assert!(encoded.starts_with("111")); // 3 leading zeros = 3 '1' chars
        let decoded = bs58_decode(&encoded).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn bs58_known_value() {
        // "Hello" → "9Ajdvzr" in base58
        assert_eq!(bs58_encode(b"Hello"), "9Ajdvzr");
        assert_eq!(bs58_decode("9Ajdvzr").unwrap(), b"Hello".to_vec());
    }

    #[test]
    fn sign_transaction_returns_64_bytes() {
        let (seed, _) = generate_keypair();
        let tx_msg = serde_json::to_vec(&serde_json::json!({
            "from": "SomePublicKey",
            "to": "AnotherPublicKey",
            "amount": 1000,
        }))
        .unwrap();

        let sig = sign_transaction(&seed, &tx_msg).unwrap();
        assert_eq!(sig.len(), 64);
    }
}
