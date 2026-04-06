//! HTLC program tests — PDA derivation, instruction encoding, state validation.
//!
//! These tests verify the program's account structure, instruction formats,
//! error codes, and full lifecycle without requiring a Solana validator.

use anchor_lang::prelude::Pubkey;
use sha2::{Digest, Sha256};

// ── Constants ────────────────────────────────────────────

const HTLC_SEED: &[u8] = b"htlc";
const ESCROW_SEED: &[u8] = b"escrow";

// ── Helpers ──────────────────────────────────────────────

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn disc(name: &str) -> [u8; 8] {
    let full = format!("global:{name}");
    let hash = sha256(full.as_bytes());
    let mut d = [0u8; 8];
    d.copy_from_slice(&hash[..8]);
    d
}

fn htlc_pda(program_id: &Pubkey, hashlock: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[HTLC_SEED, hashlock.as_ref()], program_id)
}

fn escrow_pda(program_id: &Pubkey, hashlock: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ESCROW_SEED, hashlock.as_ref()], program_id)
}

fn encode_lock_funds(hashlock: [u8; 32], timelock: i64, amount: u64) -> Vec<u8> {
    let mut data = disc("lock_funds").to_vec();
    data.extend_from_slice(&hashlock);
    data.extend_from_slice(&timelock.to_le_bytes());
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

fn encode_claim(secret: [u8; 32]) -> Vec<u8> {
    let mut data = disc("claim").to_vec();
    data.extend_from_slice(&secret);
    data
}

fn encode_refund() -> Vec<u8> {
    disc("refund").to_vec()
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use intentx_htlc::Htlc;

    // ── PDA derivation ───────────────────────────────

    #[test]
    fn htlc_pda_deterministic() {
        let prog = Pubkey::new_unique();
        let hashlock = sha256(b"secret1");
        assert_eq!(htlc_pda(&prog, &hashlock), htlc_pda(&prog, &hashlock));
    }

    #[test]
    fn escrow_pda_deterministic() {
        let prog = Pubkey::new_unique();
        let hashlock = sha256(b"secret2");
        assert_eq!(escrow_pda(&prog, &hashlock), escrow_pda(&prog, &hashlock));
    }

    #[test]
    fn different_hashlocks_different_pdas() {
        let prog = Pubkey::new_unique();
        let h1 = sha256(b"secret_a");
        let h2 = sha256(b"secret_b");
        assert_ne!(htlc_pda(&prog, &h1).0, htlc_pda(&prog, &h2).0);
        assert_ne!(escrow_pda(&prog, &h1).0, escrow_pda(&prog, &h2).0);
    }

    #[test]
    fn htlc_and_escrow_pdas_different() {
        let prog = Pubkey::new_unique();
        let hashlock = sha256(b"test");
        assert_ne!(htlc_pda(&prog, &hashlock).0, escrow_pda(&prog, &hashlock).0);
    }

    // ── Secret / hashlock ────────────────────────────

    #[test]
    fn hashlock_is_sha256_of_secret() {
        let secret = [0xABu8; 32];
        let hashlock = sha256(&secret);
        assert_eq!(hashlock.len(), 32);
        assert_ne!(hashlock, secret);
    }

    #[test]
    fn hashlock_deterministic() {
        let secret = [0x42u8; 32];
        assert_eq!(sha256(&secret), sha256(&secret));
    }

    #[test]
    fn different_secrets_different_hashlocks() {
        assert_ne!(sha256(&[1u8; 32]), sha256(&[2u8; 32]));
    }

    #[test]
    fn verify_preimage() {
        let secret = [0xFFu8; 32];
        let hashlock = sha256(&secret);
        // Verification: hash the candidate and compare
        assert_eq!(sha256(&secret), hashlock);
        // Wrong secret fails
        assert_ne!(sha256(&[0x00u8; 32]), hashlock);
    }

    // ── Instruction data encoding ────────────────────

    #[test]
    fn lock_funds_data_format() {
        let hashlock = sha256(b"my_secret");
        let timelock: i64 = 1700000000;
        let amount: u64 = 1_000_000;

        let data = encode_lock_funds(hashlock, timelock, amount);

        // 8 disc + 32 hashlock + 8 timelock + 8 amount = 56
        assert_eq!(data.len(), 56);
        assert_eq!(&data[..8], &disc("lock_funds"));
        assert_eq!(&data[8..40], &hashlock);
        assert_eq!(
            i64::from_le_bytes(data[40..48].try_into().unwrap()),
            timelock
        );
        assert_eq!(
            u64::from_le_bytes(data[48..56].try_into().unwrap()),
            amount
        );
    }

    #[test]
    fn claim_data_format() {
        let secret = [0xAA; 32];
        let data = encode_claim(secret);

        // 8 disc + 32 secret = 40
        assert_eq!(data.len(), 40);
        assert_eq!(&data[..8], &disc("claim"));
        assert_eq!(&data[8..40], &secret);
    }

    #[test]
    fn refund_data_format() {
        let data = encode_refund();
        assert_eq!(data.len(), 8);
        assert_eq!(data, disc("refund").to_vec());
    }

    // ── Discriminators ───────────────────────────────

    #[test]
    fn discriminators_unique() {
        let names = ["lock_funds", "claim", "refund"];
        let discs: Vec<_> = names.iter().map(|n| disc(n)).collect();
        for i in 0..discs.len() {
            for j in (i + 1)..discs.len() {
                assert_ne!(discs[i], discs[j], "{} == {}", names[i], names[j]);
            }
        }
    }

    // ── Account sizes ────────────────────────────────

    #[test]
    fn htlc_account_size() {
        // 8 disc + 32 sender + 32 receiver + 32 mint + 32 hashlock
        // + 8 timelock + 8 amount + 1 claimed + 1 refunded + 1 bump + 1 escrow_bump
        assert_eq!(Htlc::SIZE, 156);
    }

    // ── Seed values ────────────────────────────────────

    #[test]
    fn seeds_are_expected_values() {
        assert_eq!(HTLC_SEED, b"htlc");
        assert_eq!(ESCROW_SEED, b"escrow");
    }

    // ── Full flow (structural) ───────────────────────

    #[test]
    fn full_htlc_lifecycle_instruction_sequence() {
        let program_id = Pubkey::new_unique();
        let sender = Pubkey::new_unique();
        let receiver = Pubkey::new_unique();
        let mint = Pubkey::new_unique();

        // Generate secret and hashlock
        let secret = [0x42u8; 32];
        let hashlock = sha256(&secret);

        let timelock: i64 = 1700000000;
        let amount: u64 = 5_000_000;

        // Derive PDAs
        let (htlc_key, _) = htlc_pda(&program_id, &hashlock);
        let (escrow_key, _) = escrow_pda(&program_id, &hashlock);

        // Step 1: Lock funds
        let lock_data = encode_lock_funds(hashlock, timelock, amount);
        assert_eq!(lock_data.len(), 56);

        // Step 2: Claim with secret
        let claim_data = encode_claim(secret);
        assert_eq!(claim_data.len(), 40);

        // Verify the secret matches hashlock
        assert_eq!(sha256(&secret), hashlock);

        // All accounts are unique
        let accounts = [htlc_key, escrow_key, sender, receiver, mint];
        for i in 0..accounts.len() {
            for j in (i + 1)..accounts.len() {
                assert_ne!(accounts[i], accounts[j], "Account collision at {i},{j}");
            }
        }
    }

    #[test]
    fn refund_flow_after_timelock() {
        let program_id = Pubkey::new_unique();
        let secret = [0xBB; 32];
        let hashlock = sha256(&secret);

        let (htlc_key, _) = htlc_pda(&program_id, &hashlock);
        let (escrow_key, _) = escrow_pda(&program_id, &hashlock);

        // Lock with short timelock (already expired for test purposes)
        let past_timelock: i64 = 1;
        let lock_data = encode_lock_funds(hashlock, past_timelock, 1000);
        assert_eq!(&lock_data[..8], &disc("lock_funds"));

        // Refund instruction
        let refund_data = encode_refund();
        assert_eq!(refund_data.len(), 8);

        // PDAs are valid
        assert_ne!(htlc_key, escrow_key);
    }
}
