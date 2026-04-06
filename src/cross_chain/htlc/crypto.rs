//! HTLC cryptographic primitives: secret generation, hashing, verification.

use sha2::{Digest, Sha256};

/// A 32-byte secret used for the HTLC preimage.
pub type Secret = [u8; 32];

/// A 32-byte hash used as the HTLC lock condition.
pub type SecretHash = [u8; 32];

/// Generate a cryptographically random 32-byte secret.
pub fn generate_secret() -> Secret {
    rand::random()
}

/// Compute SHA-256(secret) to produce the hash lock.
pub fn hash_secret(secret: &Secret) -> SecretHash {
    let mut hasher = Sha256::new();
    hasher.update(secret);
    hasher.finalize().into()
}

/// Verify that a revealed secret matches the expected hash.
pub fn verify_secret(secret: &Secret, expected_hash: &SecretHash) -> bool {
    hash_secret(secret) == *expected_hash
}

/// Encode bytes as hex string for storage/display.
pub fn to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Decode hex string back to bytes.
pub fn from_hex(hex_str: &str) -> Result<Vec<u8>, String> {
    hex::decode(hex_str).map_err(|e| format!("Invalid hex: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_32_bytes() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32);
    }

    #[test]
    fn hash_is_32_bytes() {
        let secret = generate_secret();
        let hash = hash_secret(&secret);
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn hash_is_deterministic() {
        let secret = generate_secret();
        assert_eq!(hash_secret(&secret), hash_secret(&secret));
    }

    #[test]
    fn different_secrets_different_hashes() {
        let s1 = generate_secret();
        let s2 = generate_secret();
        assert_ne!(hash_secret(&s1), hash_secret(&s2));
    }

    #[test]
    fn verify_correct_secret() {
        let secret = generate_secret();
        let hash = hash_secret(&secret);
        assert!(verify_secret(&secret, &hash));
    }

    #[test]
    fn verify_wrong_secret_fails() {
        let secret = generate_secret();
        let hash = hash_secret(&secret);
        let wrong = generate_secret();
        assert!(!verify_secret(&wrong, &hash));
    }

    #[test]
    fn hex_roundtrip() {
        let secret = generate_secret();
        let hex_str = to_hex(&secret);
        assert_eq!(hex_str.len(), 64);
        let decoded = from_hex(&hex_str).unwrap();
        assert_eq!(decoded, secret.to_vec());
    }

    #[test]
    fn preimage_resistance() {
        // Hash should not be trivially reversible
        let secret = generate_secret();
        let hash = hash_secret(&secret);
        assert_ne!(secret, hash); // hash != preimage
    }
}
