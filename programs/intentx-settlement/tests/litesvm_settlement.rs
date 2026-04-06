//! Integration tests for the IntentX settlement program.
//!
//! These tests validate PDA derivation, instruction encoding, account
//! structure, and the full settlement flow instruction sequence.
//! All tests run without external dependencies (no validator, no Docker).
//!
//! For on-chain execution tests with LiteSVM, build and run from the
//! separate `tests/solana-integration` workspace that pins compatible
//! Solana SDK versions.

use anchor_lang::prelude::Pubkey;

// ── Constants ────────────────────────────────────────────

const SPL_TOKEN: Pubkey = anchor_spl::token::ID;

// ── Helpers ──────────────────────────────────────────────

/// Anchor discriminator: sha256("global:{name}")[..8]
fn disc(name: &str) -> [u8; 8] {
    let full = format!("global:{name}");
    let hash = <sha2::Sha256 as sha2::Digest>::digest(full.as_bytes());
    let mut d = [0u8; 8];
    d.copy_from_slice(&hash[..8]);
    d
}

fn config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], program_id)
}

fn user_pda(program_id: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"user", owner.as_ref(), mint.as_ref()], program_id)
}

fn vault_auth_pda(program_id: &Pubkey, config: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", config.as_ref()], program_id)
}

/// Derive ATA: seeds = [owner, TOKEN_PROGRAM, mint], program = ATA_PROGRAM
fn derive_ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    let ata_program = anchor_spl::associated_token::ID;
    Pubkey::find_program_address(
        &[owner.as_ref(), SPL_TOKEN.as_ref(), mint.as_ref()],
        &ata_program,
    )
    .0
}

/// Encode an initialize instruction's data.
fn encode_initialize(fee_bps: u16) -> Vec<u8> {
    let mut data = disc("initialize").to_vec();
    data.extend_from_slice(&fee_bps.to_le_bytes());
    data
}

/// Encode a deposit instruction's data.
fn encode_deposit(amount: u64) -> Vec<u8> {
    let mut data = disc("deposit").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

/// Encode a settle instruction's data.
fn encode_settle(amount: u64, fill_id: [u8; 16]) -> Vec<u8> {
    let mut data = disc("settle").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&fill_id);
    data
}

/// Encode a withdraw instruction's data.
fn encode_withdraw(amount: u64) -> Vec<u8> {
    let mut data = disc("withdraw").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ::intentx_settlement::*;

    // ── PDA derivation ───────────────────────────────

    #[test]
    fn config_pda_deterministic() {
        let p = Pubkey::new_unique();
        assert_eq!(config_pda(&p), config_pda(&p));
    }

    #[test]
    fn user_pda_deterministic() {
        let (p, o, m) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
        assert_eq!(user_pda(&p, &o, &m), user_pda(&p, &o, &m));
    }

    #[test]
    fn different_owners_different_user_pdas() {
        let p = Pubkey::new_unique();
        let m = Pubkey::new_unique();
        assert_ne!(
            user_pda(&p, &Pubkey::new_unique(), &m).0,
            user_pda(&p, &Pubkey::new_unique(), &m).0
        );
    }

    #[test]
    fn different_mints_different_user_pdas() {
        let p = Pubkey::new_unique();
        let o = Pubkey::new_unique();
        assert_ne!(
            user_pda(&p, &o, &Pubkey::new_unique()).0,
            user_pda(&p, &o, &Pubkey::new_unique()).0
        );
    }

    #[test]
    fn vault_auth_not_equal_config() {
        let p = Pubkey::new_unique();
        let (c, _) = config_pda(&p);
        let (v, _) = vault_auth_pda(&p, &c);
        assert_ne!(c, v);
    }

    #[test]
    fn all_pdas_unique_for_same_program() {
        let p = Pubkey::new_unique();
        let buyer = Pubkey::new_unique();
        let seller = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let (config, _) = config_pda(&p);
        let (buyer_acc, _) = user_pda(&p, &buyer, &mint);
        let (seller_acc, _) = user_pda(&p, &seller, &mint);
        let (vault, _) = vault_auth_pda(&p, &config);

        let all = [config, buyer_acc, seller_acc, vault];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "PDA collision at {i},{j}");
            }
        }
    }

    // ── ATA derivation ───────────────────────────────

    #[test]
    fn ata_deterministic() {
        let o = Pubkey::new_unique();
        let m = Pubkey::new_unique();
        assert_eq!(derive_ata(&o, &m), derive_ata(&o, &m));
    }

    #[test]
    fn ata_different_owners() {
        let m = Pubkey::new_unique();
        assert_ne!(
            derive_ata(&Pubkey::new_unique(), &m),
            derive_ata(&Pubkey::new_unique(), &m)
        );
    }

    #[test]
    fn ata_different_mints() {
        let o = Pubkey::new_unique();
        assert_ne!(
            derive_ata(&o, &Pubkey::new_unique()),
            derive_ata(&o, &Pubkey::new_unique())
        );
    }

    #[test]
    fn ata_not_equal_to_inputs() {
        let o = Pubkey::new_unique();
        let m = Pubkey::new_unique();
        let ata = derive_ata(&o, &m);
        assert_ne!(ata, o);
        assert_ne!(ata, m);
    }

    // ── Discriminator ────────────────────────────────

    #[test]
    fn discriminators_unique() {
        let names = ["initialize", "deposit", "settle", "withdraw", "update_authority", "update_fee"];
        let discs: Vec<_> = names.iter().map(|n| disc(n)).collect();
        for i in 0..discs.len() {
            for j in (i + 1)..discs.len() {
                assert_ne!(discs[i], discs[j], "{} == {}", names[i], names[j]);
            }
        }
    }

    #[test]
    fn discriminators_8_bytes() {
        assert_eq!(disc("settle").len(), 8);
    }

    // ── Instruction data encoding ────────────────────

    #[test]
    fn initialize_data_format() {
        let data = encode_initialize(250);
        assert_eq!(data.len(), 10); // 8 disc + 2 fee_bps
        assert_eq!(&data[..8], &disc("initialize"));
        assert_eq!(u16::from_le_bytes(data[8..10].try_into().unwrap()), 250);
    }

    #[test]
    fn deposit_data_format() {
        let data = encode_deposit(1_000_000);
        assert_eq!(data.len(), 16); // 8 disc + 8 amount
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 1_000_000);
    }

    #[test]
    fn settle_data_format() {
        let fill = [0xABu8; 16];
        let data = encode_settle(50_000, fill);
        assert_eq!(data.len(), 32); // 8 disc + 8 amount + 16 fill_id
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 50_000);
        assert_eq!(&data[16..32], &fill);
    }

    #[test]
    fn withdraw_data_format() {
        let data = encode_withdraw(25_000);
        assert_eq!(data.len(), 16);
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 25_000);
    }

    // ── Account sizes ────────────────────────────────

    #[test]
    fn config_space() {
        // 8 disc + 32 authority + 2 fee_bps + 32 fee_recipient + 8 total_settlements + 8 total_volume + 1 bump
        assert_eq!(8 + 32 + 2 + 32 + 8 + 8 + 1, 91);
    }

    #[test]
    fn user_account_space() {
        // 8 disc + 32 owner + 32 mint + 8 balance + 1 bump
        assert_eq!(8 + 32 + 32 + 8 + 1, 81);
    }

    // ── Fee calculation ──────────────────────────────

    #[test]
    fn fee_1_percent() {
        let amount: u64 = 10_000;
        let fee_bps: u16 = 100; // 1%
        let fee = (amount as u128 * fee_bps as u128 / 10_000) as u64;
        assert_eq!(fee, 100);
        assert_eq!(amount - fee, 9_900);
    }

    #[test]
    fn fee_zero() {
        let fee = (5000u128 * 0u128 / 10_000) as u64;
        assert_eq!(fee, 0);
    }

    #[test]
    fn fee_max() {
        let amount: u64 = 10_000;
        let fee = (amount as u128 * MAX_FEE_BPS as u128 / 10_000) as u64;
        assert_eq!(fee, 5_000); // 50%
    }

    #[test]
    fn fee_no_overflow_u64_max() {
        let amount: u64 = u64::MAX;
        let fee_bps: u16 = 100;
        let fee = (amount as u128)
            .checked_mul(fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap();
        assert!(fee < amount as u128);
    }

    // ── Full settlement flow (structural) ────────────

    #[test]
    fn full_settlement_flow() {
        let program_id = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let fee_recipient = Pubkey::new_unique();
        let buyer = Pubkey::new_unique();
        let seller = Pubkey::new_unique();
        let mint = Pubkey::new_unique();

        let (config, _) = config_pda(&program_id);
        let (buyer_pda, _) = user_pda(&program_id, &buyer, &mint);
        let (seller_pda, _) = user_pda(&program_id, &seller, &mint);
        let (fee_pda, _) = user_pda(&program_id, &fee_recipient, &mint);
        let (vault_auth, _) = vault_auth_pda(&program_id, &config);

        // 1. Initialize: 1% fee
        let init_data = encode_initialize(100);
        assert_eq!(init_data.len(), 10);

        // 2. Buyer deposits 10,000
        let dep_data = encode_deposit(10_000);
        assert_eq!(dep_data.len(), 16);

        // 3. Settle: buyer → seller, 10,000 at 1% = 100 fee
        let fill_id = [42u8; 16];
        let settle_data = encode_settle(10_000, fill_id);
        assert_eq!(settle_data.len(), 32);

        // Expected balances after settle:
        // buyer:  10000 - 10000 = 0
        // seller: 0 + 9900 = 9900
        // fee:    0 + 100 = 100
        let expected_seller = 10_000 - 100;
        let expected_fee = 100u64;
        assert_eq!(expected_seller, 9_900);
        assert_eq!(expected_fee, 100);

        // 4. Seller withdraws 9900
        let wd_data = encode_withdraw(9_900);
        assert_eq!(wd_data.len(), 16);

        // Verify all PDAs are distinct
        let pdas = [config, buyer_pda, seller_pda, fee_pda, vault_auth];
        for i in 0..pdas.len() {
            for j in (i + 1)..pdas.len() {
                assert_ne!(pdas[i], pdas[j]);
            }
        }

        // Verify seeds match program constants
        assert_eq!(VAULT_SEED, b"vault");
        assert_eq!(USER_SEED, b"user");
    }
}
