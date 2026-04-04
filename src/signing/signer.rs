use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const HEADER_SIGNATURE: &str = "x-signature";
pub const HEADER_TIMESTAMP: &str = "x-timestamp";
pub const HEADER_NONCE: &str = "x-nonce";

/// Compute HMAC-SHA256 signature for a request.
/// message = method + path + body + timestamp + nonce
pub fn compute_signature(
    secret: &str,
    method: &str,
    path: &str,
    body: &[u8],
    timestamp: i64,
    nonce: &str,
) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");

    mac.update(method.as_bytes());
    mac.update(b"|");
    mac.update(path.as_bytes());
    mac.update(b"|");
    mac.update(body);
    mac.update(b"|");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b"|");
    mac.update(nonce.as_bytes());

    hex::encode(mac.finalize().into_bytes())
}

/// Sign an outgoing request: returns (signature, timestamp, nonce) to add as headers.
pub fn sign_request(
    secret: &str,
    method: &str,
    path: &str,
    body: &[u8],
) -> (String, String, String) {
    let timestamp = chrono::Utc::now().timestamp();
    let nonce = Uuid::new_v4().to_string();
    let signature = compute_signature(secret, method, path, body, timestamp, &nonce);
    (signature, timestamp.to_string(), nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_deterministic() {
        let sig1 = compute_signature("secret", "POST", "/intents", b"{}", 1000, "nonce1");
        let sig2 = compute_signature("secret", "POST", "/intents", b"{}", 1000, "nonce1");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn different_body_different_signature() {
        let sig1 = compute_signature("secret", "POST", "/intents", b"{\"a\":1}", 1000, "n");
        let sig2 = compute_signature("secret", "POST", "/intents", b"{\"a\":2}", 1000, "n");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn different_secret_different_signature() {
        let sig1 = compute_signature("secret1", "POST", "/x", b"", 1000, "n");
        let sig2 = compute_signature("secret2", "POST", "/x", b"", 1000, "n");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn different_nonce_different_signature() {
        let sig1 = compute_signature("secret", "POST", "/x", b"", 1000, "n1");
        let sig2 = compute_signature("secret", "POST", "/x", b"", 1000, "n2");
        assert_ne!(sig1, sig2);
    }
}
