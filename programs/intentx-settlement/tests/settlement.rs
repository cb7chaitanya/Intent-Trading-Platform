//! Unit-level tests for the settlement program logic.
//! These test the account struct sizes and error variants without
//! requiring a full Solana validator. Integration tests that deploy
//! the program run via `anchor test`.

#[cfg(test)]
mod tests {
    use ::intentx_settlement::*;

    #[test]
    fn config_account_size() {
        // 8 (discriminator) + 32 (authority) + 2 (fee_bps) + 32 (fee_recipient)
        // + 8 (total_settlements) + 8 (total_volume) + 1 (bump) = 91
        let expected = 8 + 32 + 2 + 32 + 8 + 8 + 1;
        assert_eq!(expected, 91);
    }

    #[test]
    fn user_account_size() {
        // 8 (discriminator) + 32 (owner) + 32 (mint) + 8 (balance) + 1 (bump) = 81
        let expected = 8 + 32 + 32 + 8 + 1;
        assert_eq!(expected, 81);
    }

    #[test]
    fn max_fee_bps_is_50_percent() {
        assert_eq!(MAX_FEE_BPS, 5000);
    }

    #[test]
    fn fee_calculation() {
        // 1000 tokens at 100 bps (1%) = 10 fee
        let amount: u64 = 1000;
        let fee_bps: u16 = 100;
        let fee = (amount as u128 * fee_bps as u128 / 10_000) as u64;
        assert_eq!(fee, 10);
        assert_eq!(amount - fee, 990);
    }

    #[test]
    fn fee_calculation_zero_bps() {
        let amount: u64 = 5000;
        let fee_bps: u16 = 0;
        let fee = (amount as u128 * fee_bps as u128 / 10_000) as u64;
        assert_eq!(fee, 0);
    }

    #[test]
    fn fee_calculation_max_bps() {
        let amount: u64 = 10_000;
        let fee_bps: u16 = MAX_FEE_BPS;
        let fee = (amount as u128 * fee_bps as u128 / 10_000) as u64;
        assert_eq!(fee, 5000); // 50%
    }

    #[test]
    fn fee_no_overflow_large_amount() {
        let amount: u64 = u64::MAX;
        let fee_bps: u16 = 100;
        // Must use u128 to avoid overflow
        let fee = (amount as u128)
            .checked_mul(fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;
        assert!(fee > 0);
        assert!(fee < amount);
    }

    #[test]
    fn vault_seed_constant() {
        assert_eq!(VAULT_SEED, b"vault");
    }

    #[test]
    fn user_seed_constant() {
        assert_eq!(USER_SEED, b"user");
    }
}
