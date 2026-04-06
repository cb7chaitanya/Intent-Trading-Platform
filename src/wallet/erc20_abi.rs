//! ERC-20 ABI encoding for Ethereum contract calls.
//!
//! Implements manual Solidity ABI encoding without ethers-core dependency.
//! Each function is encoded as: selector (4 bytes) + arguments (32 bytes each).

use sha2::{Digest, Sha256};

// ── Keccak-256 substitute ────────────────────────────────
//
// EVM uses keccak256 for function selectors. We approximate with SHA-256
// here. In production, add the `tiny-keccak` crate for real keccak256.
// The selector values below are hardcoded from the real keccak256 output
// so the encoded calldata is correct regardless of hash function used
// internally.

/// ERC-20 function selectors (first 4 bytes of keccak256 of signature).
/// These are well-known constants — no need to compute at runtime.
pub const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)
pub const APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3]; // approve(address,uint256)
pub const TRANSFER_FROM_SELECTOR: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd]; // transferFrom(address,address,uint256)
pub const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)

// ── ABI encoding helpers ─────────────────────────────────

/// Left-pad a 20-byte Ethereum address to 32 bytes.
fn encode_address(addr: &[u8; 20]) -> [u8; 32] {
    let mut padded = [0u8; 32];
    padded[12..].copy_from_slice(addr);
    padded
}

/// Encode a u256 value as 32 bytes big-endian.
fn encode_uint256(value: u128) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[16..].copy_from_slice(&value.to_be_bytes());
    buf
}

/// Parse a hex address string (with or without 0x prefix) into 20 bytes.
pub fn parse_address(addr: &str) -> Result<[u8; 20], String> {
    let clean = addr.strip_prefix("0x").unwrap_or(addr);
    if clean.len() != 40 {
        return Err(format!("Address must be 40 hex chars, got {}", clean.len()));
    }
    let bytes = hex::decode(clean).map_err(|e| format!("Invalid hex: {e}"))?;
    let mut result = [0u8; 20];
    result.copy_from_slice(&bytes);
    Ok(result)
}

// ── ERC-20 calldata builders ─────────────────────────────

/// Encode `transfer(address to, uint256 amount)` calldata.
///
/// Used for sending ERC-20 tokens from the caller to a recipient.
pub fn encode_transfer(to: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut data = Vec::with_capacity(68); // 4 + 32 + 32
    data.extend_from_slice(&TRANSFER_SELECTOR);
    data.extend_from_slice(&encode_address(to));
    data.extend_from_slice(&encode_uint256(amount));
    data
}

/// Encode `approve(address spender, uint256 amount)` calldata.
///
/// Used to grant an allowance to a spender (e.g., settlement contract).
pub fn encode_approve(spender: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut data = Vec::with_capacity(68);
    data.extend_from_slice(&APPROVE_SELECTOR);
    data.extend_from_slice(&encode_address(spender));
    data.extend_from_slice(&encode_uint256(amount));
    data
}

/// Encode `transferFrom(address from, address to, uint256 amount)` calldata.
///
/// Used when the settlement contract moves tokens on behalf of users
/// (requires prior approval).
pub fn encode_transfer_from(from: &[u8; 20], to: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut data = Vec::with_capacity(100); // 4 + 32 + 32 + 32
    data.extend_from_slice(&TRANSFER_FROM_SELECTOR);
    data.extend_from_slice(&encode_address(from));
    data.extend_from_slice(&encode_address(to));
    data.extend_from_slice(&encode_uint256(amount));
    data
}

/// Encode `balanceOf(address account)` calldata.
///
/// Used for eth_call to query token balance.
pub fn encode_balance_of(account: &[u8; 20]) -> Vec<u8> {
    let mut data = Vec::with_capacity(36); // 4 + 32
    data.extend_from_slice(&BALANCE_OF_SELECTOR);
    data.extend_from_slice(&encode_address(account));
    data
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> [u8; 20] {
        let mut a = [0u8; 20];
        a[19] = b;
        a
    }

    // ── Selector constants ───────────────────────────

    #[test]
    fn transfer_selector_is_correct() {
        // keccak256("transfer(address,uint256)") = 0xa9059cbb...
        assert_eq!(TRANSFER_SELECTOR, [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn approve_selector_is_correct() {
        // keccak256("approve(address,uint256)") = 0x095ea7b3...
        assert_eq!(APPROVE_SELECTOR, [0x09, 0x5e, 0xa7, 0xb3]);
    }

    #[test]
    fn transfer_from_selector_is_correct() {
        // keccak256("transferFrom(address,address,uint256)") = 0x23b872dd...
        assert_eq!(TRANSFER_FROM_SELECTOR, [0x23, 0xb8, 0x72, 0xdd]);
    }

    #[test]
    fn balance_of_selector_is_correct() {
        // keccak256("balanceOf(address)") = 0x70a08231...
        assert_eq!(BALANCE_OF_SELECTOR, [0x70, 0xa0, 0x82, 0x31]);
    }

    // ── Address encoding ─────────────────────────────

    #[test]
    fn address_left_padded_to_32_bytes() {
        let a = addr(0xAB);
        let encoded = encode_address(&a);
        assert_eq!(encoded.len(), 32);
        // First 12 bytes are zero
        assert_eq!(&encoded[..12], &[0u8; 12]);
        // Last 20 bytes are the address
        assert_eq!(&encoded[12..], a.as_slice());
    }

    // ── uint256 encoding ─────────────────────────────

    #[test]
    fn uint256_encoding_big_endian() {
        let encoded = encode_uint256(1000);
        assert_eq!(encoded.len(), 32);
        // 1000 = 0x3E8
        assert_eq!(encoded[31], 0xE8);
        assert_eq!(encoded[30], 0x03);
        assert_eq!(&encoded[..30], &[0u8; 30]); // leading zeros
    }

    #[test]
    fn uint256_zero() {
        let encoded = encode_uint256(0);
        assert_eq!(encoded, [0u8; 32]);
    }

    #[test]
    fn uint256_max_u128() {
        let encoded = encode_uint256(u128::MAX);
        // u128::MAX in big-endian occupies bytes 16..32
        assert_eq!(&encoded[..16], &[0u8; 16]);
        assert_eq!(&encoded[16..], &u128::MAX.to_be_bytes());
    }

    // ── transfer() encoding ──────────────────────────

    #[test]
    fn transfer_calldata_length() {
        let data = encode_transfer(&addr(1), 1000);
        assert_eq!(data.len(), 68); // 4 + 32 + 32
    }

    #[test]
    fn transfer_starts_with_selector() {
        let data = encode_transfer(&addr(1), 1000);
        assert_eq!(&data[..4], &TRANSFER_SELECTOR);
    }

    #[test]
    fn transfer_encodes_recipient() {
        let to = addr(0xBE);
        let data = encode_transfer(&to, 500);
        // Address at bytes 4..36 (left-padded)
        assert_eq!(data[35], 0xBE);
        assert_eq!(&data[4..16], &[0u8; 12]); // padding
    }

    #[test]
    fn transfer_encodes_amount() {
        let data = encode_transfer(&addr(1), 1_000_000);
        // Amount at bytes 36..68
        let amount_bytes = &data[36..68];
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&amount_bytes[16..32]);
        let decoded = u128::from_be_bytes(buf);
        assert_eq!(decoded, 1_000_000);
    }

    // ── approve() encoding ───────────────────────────

    #[test]
    fn approve_calldata_correct() {
        let spender = addr(0xAA);
        let data = encode_approve(&spender, u128::MAX);
        assert_eq!(data.len(), 68);
        assert_eq!(&data[..4], &APPROVE_SELECTOR);
        assert_eq!(data[35], 0xAA); // spender
        // Max approval = all 1s in amount
        assert_eq!(&data[52..68], &u128::MAX.to_be_bytes());
    }

    // ── transferFrom() encoding ──────────────────────

    #[test]
    fn transfer_from_calldata_length() {
        let data = encode_transfer_from(&addr(1), &addr(2), 500);
        assert_eq!(data.len(), 100); // 4 + 32 + 32 + 32
    }

    #[test]
    fn transfer_from_encodes_all_args() {
        let from = addr(0x11);
        let to = addr(0x22);
        let data = encode_transfer_from(&from, &to, 99);

        assert_eq!(&data[..4], &TRANSFER_FROM_SELECTOR);
        assert_eq!(data[35], 0x11);  // from
        assert_eq!(data[67], 0x22);  // to
        assert_eq!(data[99], 99);    // amount (last byte)
    }

    // ── balanceOf() encoding ─────────────────────────

    #[test]
    fn balance_of_calldata() {
        let account = addr(0xFF);
        let data = encode_balance_of(&account);
        assert_eq!(data.len(), 36); // 4 + 32
        assert_eq!(&data[..4], &BALANCE_OF_SELECTOR);
        assert_eq!(data[35], 0xFF);
    }

    // ── parse_address ────────────────────────────────

    #[test]
    fn parse_address_with_prefix() {
        let a = parse_address("0x0000000000000000000000000000000000000001").unwrap();
        assert_eq!(a[19], 1);
    }

    #[test]
    fn parse_address_without_prefix() {
        let a = parse_address("0000000000000000000000000000000000000002").unwrap();
        assert_eq!(a[19], 2);
    }

    #[test]
    fn parse_address_wrong_length() {
        assert!(parse_address("0x1234").is_err());
    }

    // ── Real-world calldata verification ─────────────

    #[test]
    fn transfer_1_usdc_matches_expected() {
        // USDC has 6 decimals, so 1 USDC = 1_000_000
        let recipient = parse_address("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045").unwrap();
        let data = encode_transfer(&recipient, 1_000_000);

        // Verify selector
        assert_eq!(hex::encode(&data[..4]), "a9059cbb");

        // Verify the calldata is exactly 68 bytes
        assert_eq!(data.len(), 68);

        // Verify recipient is correctly padded
        let expected_addr_hex = "000000000000000000000000d8da6bf26964af9d7eed9e03e53415d37aa96045";
        assert_eq!(hex::encode(&data[4..36]), expected_addr_hex);

        // Verify amount: 1_000_000 = 0xF4240
        let amount_hex = hex::encode(&data[36..68]);
        assert!(amount_hex.ends_with("0f4240"));
    }
}
