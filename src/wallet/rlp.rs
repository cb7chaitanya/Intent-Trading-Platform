//! Recursive Length Prefix (RLP) encoding for Ethereum transactions.
//!
//! Implements the RLP specification from the Ethereum Yellow Paper (Appendix B).
//!
//! Encoding rules:
//! - Single byte 0x00..=0x7f: encoded as itself
//! - String 0..=55 bytes:  0x80 + len, then bytes
//! - String >55 bytes:     0xb7 + len_of_len, then len (big-endian), then bytes
//! - List payload 0..=55:  0xc0 + len, then concatenated items
//! - List payload >55:     0xf7 + len_of_len, then len (big-endian), then items

// ── Core RLP encoding ────────────────────────────────────

/// Encode a byte slice as an RLP string.
pub fn encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.len() == 1 && data[0] <= 0x7f {
        // Single byte in [0x00, 0x7f] range — encoded as itself
        return vec![data[0]];
    }

    if data.len() <= 55 {
        // Short string: 0x80 + len prefix
        let mut out = Vec::with_capacity(1 + data.len());
        out.push(0x80 + data.len() as u8);
        out.extend_from_slice(data);
        out
    } else {
        // Long string: 0xb7 + len_of_len, then len in big-endian, then data
        let len_bytes = to_be_bytes_trimmed(data.len() as u64);
        let mut out = Vec::with_capacity(1 + len_bytes.len() + data.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(data);
        out
    }
}

/// Encode an RLP list from already-encoded items.
pub fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.iter().flat_map(|i| i.iter().copied()).collect();
    wrap_list(&payload)
}

/// Wrap raw payload bytes as an RLP list.
fn wrap_list(payload: &[u8]) -> Vec<u8> {
    if payload.len() <= 55 {
        let mut out = Vec::with_capacity(1 + payload.len());
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(payload);
        out
    } else {
        let len_bytes = to_be_bytes_trimmed(payload.len() as u64);
        let mut out = Vec::with_capacity(1 + len_bytes.len() + payload.len());
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(payload);
        out
    }
}

/// Encode a u64 as an RLP integer (big-endian, no leading zeros).
pub fn encode_u64(val: u64) -> Vec<u8> {
    if val == 0 {
        return encode_bytes(&[]);
    }
    let bytes = to_be_bytes_trimmed(val);
    encode_bytes(&bytes)
}

/// Encode a u128 as an RLP integer.
pub fn encode_u128(val: u128) -> Vec<u8> {
    if val == 0 {
        return encode_bytes(&[]);
    }
    let be = val.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(be.len());
    encode_bytes(&be[start..])
}

/// Encode a U256 (represented as [u8; 32]) as an RLP integer.
/// Strips leading zeros.
pub fn encode_u256(val: &[u8; 32]) -> Vec<u8> {
    let start = val.iter().position(|&b| b != 0).unwrap_or(32);
    if start == 32 {
        return encode_bytes(&[]);
    }
    encode_bytes(&val[start..])
}

/// Encode an empty byte array (used for empty fields).
pub fn encode_empty() -> Vec<u8> {
    vec![0x80]
}

/// Encode a 20-byte Ethereum address.
pub fn encode_address(addr: &[u8; 20]) -> Vec<u8> {
    encode_bytes(addr)
}

// ── Unsigned transaction encoding ────────────────────────

/// Fields for a legacy (pre-EIP-155) or EIP-155 unsigned transaction.
pub struct LegacyTxFields {
    pub nonce: u64,
    pub gas_price: u64,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub chain_id: u64,
}

/// Encode a legacy unsigned transaction for signing (EIP-155).
///
/// For signing: RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])
/// The signed tx replaces the last 3 fields with v, r, s.
pub fn encode_legacy_unsigned(tx: &LegacyTxFields) -> Vec<u8> {
    let items = vec![
        encode_u64(tx.nonce),
        encode_u64(tx.gas_price),
        encode_u64(tx.gas_limit),
        encode_address(&tx.to),
        encode_u128(tx.value),
        encode_bytes(&tx.data),
        encode_u64(tx.chain_id), // v = chainId for EIP-155
        encode_empty(),          // r = 0
        encode_empty(),          // s = 0
    ];
    encode_list(&items)
}

/// Encode a legacy signed transaction.
///
/// RLP([nonce, gasPrice, gasLimit, to, value, data, v, r, s])
pub fn encode_legacy_signed(tx: &LegacyTxFields, v: u64, r: &[u8; 32], s: &[u8; 32]) -> Vec<u8> {
    let items = vec![
        encode_u64(tx.nonce),
        encode_u64(tx.gas_price),
        encode_u64(tx.gas_limit),
        encode_address(&tx.to),
        encode_u128(tx.value),
        encode_bytes(&tx.data),
        encode_u64(v),
        encode_u256(r),
        encode_u256(s),
    ];
    encode_list(&items)
}

/// Fields for an EIP-1559 (type 2) transaction.
pub struct Eip1559TxFields {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    // access_list is always empty for our use case
}

/// Encode an EIP-1559 unsigned transaction for signing.
///
/// Signing payload: 0x02 || RLP([chainId, nonce, maxPriorityFeePerGas,
///   maxFeePerGas, gasLimit, to, value, data, accessList])
pub fn encode_eip1559_unsigned(tx: &Eip1559TxFields) -> Vec<u8> {
    let items = vec![
        encode_u64(tx.chain_id),
        encode_u64(tx.nonce),
        encode_u64(tx.max_priority_fee_per_gas),
        encode_u64(tx.max_fee_per_gas),
        encode_u64(tx.gas_limit),
        encode_address(&tx.to),
        encode_u128(tx.value),
        encode_bytes(&tx.data),
        encode_list(&[]), // empty access list
    ];

    let rlp = encode_list(&items);

    // Type 2 envelope: 0x02 || RLP(...)
    let mut out = Vec::with_capacity(1 + rlp.len());
    out.push(0x02);
    out.extend_from_slice(&rlp);
    out
}

/// Encode an EIP-1559 signed transaction.
///
/// 0x02 || RLP([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
///   gasLimit, to, value, data, accessList, yParity, r, s])
pub fn encode_eip1559_signed(
    tx: &Eip1559TxFields,
    y_parity: u64,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Vec<u8> {
    let items = vec![
        encode_u64(tx.chain_id),
        encode_u64(tx.nonce),
        encode_u64(tx.max_priority_fee_per_gas),
        encode_u64(tx.max_fee_per_gas),
        encode_u64(tx.gas_limit),
        encode_address(&tx.to),
        encode_u128(tx.value),
        encode_bytes(&tx.data),
        encode_list(&[]), // empty access list
        encode_u64(y_parity),
        encode_u256(r),
        encode_u256(s),
    ];

    let rlp = encode_list(&items);
    let mut out = Vec::with_capacity(1 + rlp.len());
    out.push(0x02);
    out.extend_from_slice(&rlp);
    out
}

// ── Internal helpers ─────────────────────────────────────

/// Convert u64 to big-endian bytes, stripping leading zeros.
fn to_be_bytes_trimmed(val: u64) -> Vec<u8> {
    let be = val.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(be.len());
    be[start..].to_vec()
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── RLP primitive encoding (Yellow Paper examples) ────

    #[test]
    fn encode_single_byte() {
        // "a" (0x61) → 0x61  (single byte, no prefix)
        assert_eq!(encode_bytes(&[0x61]), vec![0x61]);
    }

    #[test]
    fn encode_single_byte_zero() {
        // 0x00 is NOT in [0x00, 0x7f] for the single-byte rule because
        // 0x00 ≤ 0x7f, so it IS encoded as itself
        assert_eq!(encode_bytes(&[0x00]), vec![0x00]);
    }

    #[test]
    fn encode_empty_string() {
        // "" → 0x80
        assert_eq!(encode_bytes(&[]), vec![0x80]);
    }

    #[test]
    fn encode_short_string() {
        // "dog" → [0x83, 'd', 'o', 'g']
        let dog = b"dog";
        let encoded = encode_bytes(dog);
        assert_eq!(encoded, vec![0x83, 0x64, 0x6f, 0x67]);
    }

    #[test]
    fn encode_55_byte_string() {
        // Exactly 55 bytes → short string (0x80 + 55 = 0xb7)
        let data = vec![0xAA; 55];
        let encoded = encode_bytes(&data);
        assert_eq!(encoded[0], 0xb7);
        assert_eq!(encoded.len(), 56);
    }

    #[test]
    fn encode_56_byte_string() {
        // 56 bytes → long string: 0xb8, 0x38, then data
        let data = vec![0xBB; 56];
        let encoded = encode_bytes(&data);
        assert_eq!(encoded[0], 0xb8); // 0xb7 + 1 (1 byte for length)
        assert_eq!(encoded[1], 56);   // length
        assert_eq!(encoded.len(), 2 + 56);
    }

    #[test]
    fn encode_long_string_256_bytes() {
        let data = vec![0xCC; 256];
        let encoded = encode_bytes(&data);
        assert_eq!(encoded[0], 0xb9); // 0xb7 + 2 (2 bytes for length)
        assert_eq!(encoded[1], 0x01); // 256 big-endian
        assert_eq!(encoded[2], 0x00);
        assert_eq!(encoded.len(), 3 + 256);
    }

    // ── Integer encoding ─────────────────────────────

    #[test]
    fn encode_integer_zero() {
        // 0 → 0x80 (empty string)
        assert_eq!(encode_u64(0), vec![0x80]);
    }

    #[test]
    fn encode_integer_small() {
        // 1 → 0x01 (single byte)
        assert_eq!(encode_u64(1), vec![0x01]);
    }

    #[test]
    fn encode_integer_127() {
        // 127 (0x7f) → 0x7f (single byte)
        assert_eq!(encode_u64(127), vec![0x7f]);
    }

    #[test]
    fn encode_integer_128() {
        // 128 (0x80) → [0x81, 0x80] (short string, 1 byte)
        assert_eq!(encode_u64(128), vec![0x81, 0x80]);
    }

    #[test]
    fn encode_integer_1024() {
        // 1024 = 0x0400 → [0x82, 0x04, 0x00]
        assert_eq!(encode_u64(1024), vec![0x82, 0x04, 0x00]);
    }

    #[test]
    fn encode_integer_no_leading_zeros() {
        // 256 = 0x0100 → [0x82, 0x01, 0x00] (not [0x88, 0x00...])
        let encoded = encode_u64(256);
        assert_eq!(encoded, vec![0x82, 0x01, 0x00]);
    }

    // ── List encoding ────────────────────────────────

    #[test]
    fn encode_empty_list() {
        // [] → 0xc0
        assert_eq!(encode_list(&[]), vec![0xc0]);
    }

    #[test]
    fn encode_list_of_strings() {
        // ["cat", "dog"]
        let items = vec![
            encode_bytes(b"cat"),
            encode_bytes(b"dog"),
        ];
        let encoded = encode_list(&items);
        // List payload: [0x83, 'c','a','t', 0x83, 'd','o','g'] = 8 bytes
        // Prefix: 0xc0 + 8 = 0xc8
        assert_eq!(encoded[0], 0xc8);
        assert_eq!(encoded.len(), 9);
    }

    #[test]
    fn encode_nested_list() {
        // [[], [[]], [[], [[]]]]
        // This is the canonical RLP test case
        let inner_empty = encode_list(&[]);
        let inner_nested = encode_list(&[encode_list(&[])]);
        let inner_complex = encode_list(&[
            encode_list(&[]),
            encode_list(&[encode_list(&[])]),
        ]);
        let encoded = encode_list(&[inner_empty, inner_nested, inner_complex]);
        // Expected: 0xc7, 0xc0, 0xc1, 0xc0, 0xc3, 0xc0, 0xc1, 0xc0
        assert_eq!(encoded, vec![0xc7, 0xc0, 0xc1, 0xc0, 0xc3, 0xc0, 0xc1, 0xc0]);
    }

    #[test]
    fn encode_long_list() {
        // List with payload > 55 bytes: 30 two-byte integers = 90 bytes payload
        let items: Vec<Vec<u8>> = (200u64..230).map(|i| encode_u64(i * 1000)).collect();
        let payload_len: usize = items.iter().map(|i| i.len()).sum();
        assert!(payload_len > 55, "payload must be >55 for long list, got {payload_len}");
        let encoded = encode_list(&items);
        assert!(encoded[0] > 0xf7); // long list prefix
    }

    // ── U256 encoding ────────────────────────────────

    #[test]
    fn encode_u256_zero() {
        let zero = [0u8; 32];
        assert_eq!(encode_u256(&zero), vec![0x80]);
    }

    #[test]
    fn encode_u256_one() {
        let mut val = [0u8; 32];
        val[31] = 1;
        assert_eq!(encode_u256(&val), vec![0x01]);
    }

    #[test]
    fn encode_u256_large() {
        let mut val = [0u8; 32];
        val[0] = 0xFF;
        val[31] = 0x01;
        let encoded = encode_u256(&val);
        // 32 bytes, all significant
        assert_eq!(encoded[0], 0x80 + 32);
        assert_eq!(encoded.len(), 33);
    }

    // ── Address encoding ─────────────────────────────

    #[test]
    fn encode_address_20_bytes() {
        let addr = [0xAA; 20];
        let encoded = encode_address(&addr);
        assert_eq!(encoded[0], 0x80 + 20); // 0x94
        assert_eq!(encoded.len(), 21);
    }

    // ── Legacy transaction encoding ──────────────────

    #[test]
    fn legacy_unsigned_is_valid_rlp_list() {
        let tx = LegacyTxFields {
            nonce: 9,
            gas_price: 20_000_000_000, // 20 gwei
            gas_limit: 21_000,
            to: [0xBB; 20],
            value: 1_000_000_000_000_000_000, // 1 ETH
            data: vec![],
            chain_id: 1,
        };
        let encoded = encode_legacy_unsigned(&tx);
        // Must start with list prefix
        assert!(encoded[0] >= 0xc0);
    }

    #[test]
    fn legacy_unsigned_eip155_includes_chain_id() {
        let tx = LegacyTxFields {
            nonce: 0,
            gas_price: 0,
            gas_limit: 21_000,
            to: [0; 20],
            value: 0,
            data: vec![],
            chain_id: 1,
        };
        let encoded = encode_legacy_unsigned(&tx);

        // The encoded list should contain chain_id=1 followed by two 0x80 (empty)
        // near the end. Find 0x01, 0x80, 0x80 pattern.
        let data = &encoded[1..]; // skip list prefix
        let tail = &data[data.len() - 3..];
        assert_eq!(tail, &[0x01, 0x80, 0x80]); // chainId=1, r=0, s=0
    }

    #[test]
    fn legacy_signed_has_v_r_s() {
        let tx = LegacyTxFields {
            nonce: 0,
            gas_price: 1,
            gas_limit: 21_000,
            to: [0x11; 20],
            value: 0,
            data: vec![],
            chain_id: 1,
        };
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r[0] = 0xAA;
        s[0] = 0xBB;
        let v = 37; // EIP-155: chainId * 2 + 35

        let encoded = encode_legacy_signed(&tx, v, &r, &s);
        assert!(encoded[0] >= 0xc0);
        // Encoded tx should contain r and s bytes
        assert!(encoded.windows(1).any(|w| w[0] == 0xAA));
        assert!(encoded.windows(1).any(|w| w[0] == 0xBB));
    }

    // ── EIP-1559 transaction encoding ────────────────

    #[test]
    fn eip1559_unsigned_starts_with_0x02() {
        let tx = Eip1559TxFields {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 2_000_000_000,
            max_fee_per_gas: 50_000_000_000,
            gas_limit: 65_000,
            to: [0xCC; 20],
            value: 0,
            data: vec![],
        };
        let encoded = encode_eip1559_unsigned(&tx);
        assert_eq!(encoded[0], 0x02);
        // Byte after 0x02 should be an RLP list prefix
        assert!(encoded[1] >= 0xc0);
    }

    #[test]
    fn eip1559_signed_starts_with_0x02() {
        let tx = Eip1559TxFields {
            chain_id: 1,
            nonce: 5,
            max_priority_fee_per_gas: 1_500_000_000,
            max_fee_per_gas: 30_000_000_000,
            gas_limit: 65_000,
            to: [0xDD; 20],
            value: 0,
            data: vec![0x01, 0x02, 0x03],
        };
        let r = [0xAA; 32];
        let s = [0xBB; 32];

        let encoded = encode_eip1559_signed(&tx, 1, &r, &s);
        assert_eq!(encoded[0], 0x02);
    }

    #[test]
    fn eip1559_signed_longer_than_unsigned() {
        let tx = Eip1559TxFields {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 21_000,
            to: [0; 20],
            value: 0,
            data: vec![],
        };
        let unsigned = encode_eip1559_unsigned(&tx);
        let signed = encode_eip1559_signed(&tx, 0, &[0; 32], &[0; 32]);
        // Signed version has yParity, r, s appended
        assert!(signed.len() > unsigned.len());
    }

    // ── Known Ethereum RLP test vectors ──────────────

    #[test]
    fn rlp_the_yellow_paper_example() {
        // From the Yellow Paper: "Lorem ipsum dolor sit amet..."
        // The string "Lorem ipsum dolor sit amet, consectetur adipisicing elit"
        // is 56 bytes — long string encoding.
        let data = b"Lorem ipsum dolor sit amet, consectetur adipisicing elit";
        assert_eq!(data.len(), 56);
        let encoded = encode_bytes(data);
        assert_eq!(encoded[0], 0xb8); // 0xb7 + 1
        assert_eq!(encoded[1], 56);
        assert_eq!(&encoded[2..], data.as_slice());
    }

    #[test]
    fn rlp_set_of_three() {
        // The set theoretical representation of three:
        // [[], [[]], [[], [[]]]]
        let encoded = encode_list(&[
            encode_list(&[]),
            encode_list(&[encode_list(&[])]),
            encode_list(&[encode_list(&[]), encode_list(&[encode_list(&[])])]),
        ]);
        let expected = hex::decode("c7c0c1c0c3c0c1c0").unwrap();
        assert_eq!(encoded, expected);
    }

    #[test]
    fn rlp_encode_15() {
        // 15 (0x0f) → 0x0f
        assert_eq!(encode_u64(15), vec![0x0f]);
    }

    #[test]
    fn rlp_encode_1024_hex() {
        // 1024 → 0x820400
        let encoded = encode_u64(1024);
        let expected = hex::decode("820400").unwrap();
        assert_eq!(encoded, expected);
    }

    // ── Round-trip sanity checks ─────────────────────

    #[test]
    fn eip1559_erc20_transfer_tx() {
        // Simulates an ERC-20 transfer(address,uint256) on mainnet
        let calldata = {
            let mut d = Vec::with_capacity(68);
            d.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]); // transfer selector
            d.extend_from_slice(&[0u8; 12]);                  // address padding
            d.extend_from_slice(&[0xBE; 20]);                 // recipient
            d.extend_from_slice(&[0u8; 24]);                  // amount padding
            d.extend_from_slice(&1_000_000u64.to_be_bytes()); // 1 USDC
            d
        };

        let tx = Eip1559TxFields {
            chain_id: 1,
            nonce: 42,
            max_priority_fee_per_gas: 2_000_000_000,
            max_fee_per_gas: 50_000_000_000,
            gas_limit: 65_000,
            to: [0xA0; 20], // token contract
            value: 0,
            data: calldata,
        };

        let unsigned = encode_eip1559_unsigned(&tx);
        assert_eq!(unsigned[0], 0x02);
        // Should be a reasonable size for an ERC-20 transfer
        assert!(unsigned.len() > 80 && unsigned.len() < 200);

        let r = [0x11; 32];
        let s = [0x22; 32];
        let signed = encode_eip1559_signed(&tx, 0, &r, &s);
        assert_eq!(signed[0], 0x02);
        // Signed adds ~67 bytes (yParity + r + s with RLP headers)
        assert!(signed.len() > unsigned.len() + 60);
    }
}
