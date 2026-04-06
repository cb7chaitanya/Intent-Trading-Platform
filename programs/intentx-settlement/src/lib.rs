use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("11111111111111111111111111111111");

// ── Constants ────────────────────────────────────────────

/// Seed for deriving the vault PDA.
pub const VAULT_SEED: &[u8] = b"vault";

/// Seed for deriving user account PDAs.
pub const USER_SEED: &[u8] = b"user";

/// Maximum basis-point fee (50% = 5000 bps — sanity cap).
pub const MAX_FEE_BPS: u16 = 5000;

// ── Program ──────────────────────────────────────────────

#[program]
pub mod intentx_settlement {
    use super::*;

    /// One-time program initialisation: set the authority that may
    /// execute settlements, and create the global vault config.
    pub fn initialize(ctx: Context<Initialize>, fee_bps: u16) -> Result<()> {
        require!(fee_bps <= MAX_FEE_BPS, SettlementError::FeeTooHigh);

        let config = &mut ctx.accounts.config;
        config.authority = ctx.accounts.authority.key();
        config.fee_bps = fee_bps;
        config.fee_recipient = ctx.accounts.fee_recipient.key();
        config.total_settlements = 0;
        config.total_volume = 0;
        config.bump = ctx.bumps.config;

        emit!(ConfigInitialized {
            authority: config.authority,
            fee_bps,
        });

        Ok(())
    }

    /// Deposit SPL tokens from the user's wallet into the program vault.
    /// Creates the user's on-chain account if it doesn't exist yet.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);

        // Transfer tokens: user wallet → vault token account
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::transfer(cpi_ctx, amount)?;

        // Update user's tracked balance
        let user_account = &mut ctx.accounts.user_account;
        user_account.balance = user_account
            .balance
            .checked_add(amount)
            .ok_or(SettlementError::Overflow)?;

        emit!(DepositEvent {
            user: ctx.accounts.user.key(),
            mint: ctx.accounts.mint.key(),
            amount,
            new_balance: user_account.balance,
        });

        Ok(())
    }

    /// Execute a settlement between buyer and seller.
    /// Only callable by the program authority (backend signer).
    ///
    /// Flow:
    /// 1. Debit buyer's tracked balance
    /// 2. Credit seller's tracked balance (minus fee)
    /// 3. Credit fee recipient's tracked balance
    /// 4. Update global stats
    pub fn settle(
        ctx: Context<Settle>,
        amount: u64,
        fill_id: [u8; 16],
    ) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);

        let config = &ctx.accounts.config;
        let fee = (amount as u128)
            .checked_mul(config.fee_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;
        let seller_receives = amount
            .checked_sub(fee)
            .ok_or(SettlementError::Overflow)?;

        // Debit buyer
        let buyer = &mut ctx.accounts.buyer_account;
        require!(buyer.balance >= amount, SettlementError::InsufficientBalance);
        buyer.balance = buyer
            .balance
            .checked_sub(amount)
            .ok_or(SettlementError::Overflow)?;

        // Credit seller
        let seller = &mut ctx.accounts.seller_account;
        seller.balance = seller
            .balance
            .checked_add(seller_receives)
            .ok_or(SettlementError::Overflow)?;

        // Credit fee recipient
        if fee > 0 {
            let fee_account = &mut ctx.accounts.fee_account;
            fee_account.balance = fee_account
                .balance
                .checked_add(fee)
                .ok_or(SettlementError::Overflow)?;
        }

        // Update global stats
        let config = &mut ctx.accounts.config;
        config.total_settlements = config
            .total_settlements
            .checked_add(1)
            .ok_or(SettlementError::Overflow)?;
        config.total_volume = config
            .total_volume
            .checked_add(amount)
            .ok_or(SettlementError::Overflow)?;

        emit!(SettlementEvent {
            fill_id,
            buyer: ctx.accounts.buyer_account.owner,
            seller: ctx.accounts.seller_account.owner,
            mint: ctx.accounts.mint.key(),
            amount,
            fee,
            seller_receives,
        });

        Ok(())
    }

    /// Withdraw SPL tokens from the vault back to the user's wallet.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);

        let user_account = &mut ctx.accounts.user_account;
        require!(
            user_account.balance >= amount,
            SettlementError::InsufficientBalance
        );
        user_account.balance = user_account
            .balance
            .checked_sub(amount)
            .ok_or(SettlementError::Overflow)?;

        // Transfer tokens: vault → user wallet (PDA-signed)
        let config_key = ctx.accounts.config.key();
        let vault_bump = ctx.bumps.vault_authority;
        let vault_seeds: &[&[u8]] = &[
            VAULT_SEED,
            config_key.as_ref(),
            &[vault_bump],
        ];
        let signer_seeds = &[vault_seeds];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: ctx.accounts.user_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer_seeds,
        );
        token::transfer(cpi_ctx, amount)?;

        emit!(WithdrawEvent {
            user: ctx.accounts.user.key(),
            mint: ctx.accounts.mint.key(),
            amount,
            remaining_balance: user_account.balance,
        });

        Ok(())
    }

    /// Update the settlement authority (admin only).
    pub fn update_authority(ctx: Context<UpdateAuthority>, new_authority: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let old = config.authority;
        config.authority = new_authority;

        emit!(AuthorityUpdated {
            old_authority: old,
            new_authority,
        });

        Ok(())
    }

    /// Update the fee rate (admin only).
    pub fn update_fee(ctx: Context<UpdateAuthority>, new_fee_bps: u16) -> Result<()> {
        require!(new_fee_bps <= MAX_FEE_BPS, SettlementError::FeeTooHigh);
        ctx.accounts.config.fee_bps = new_fee_bps;

        emit!(FeeUpdated { fee_bps: new_fee_bps });

        Ok(())
    }
}

// ── Accounts ─────────────────────────────────────────────

/// Global program configuration — one per program instance.
#[account]
pub struct Config {
    /// Backend signer authorized to execute settlements.
    pub authority: Pubkey,
    /// Fee in basis points (100 = 1%).
    pub fee_bps: u16,
    /// Account that receives settlement fees.
    pub fee_recipient: Pubkey,
    /// Lifetime settlement count.
    pub total_settlements: u64,
    /// Lifetime volume settled.
    pub total_volume: u64,
    /// PDA bump.
    pub bump: u8,
}

/// Per-user balance tracking for a specific token mint.
#[account]
pub struct UserAccount {
    /// The user's wallet pubkey.
    pub owner: Pubkey,
    /// The SPL token mint this account tracks.
    pub mint: Pubkey,
    /// Current balance held in the vault.
    pub balance: u64,
    /// PDA bump.
    pub bump: u8,
}

// ── Instruction Contexts ─────────────────────────────────

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 2 + 32 + 8 + 8 + 1,
        seeds = [b"config"],
        bump,
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: stored as fee destination, validated at settlement time.
    pub fee_recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 32 + 32 + 8 + 1,
        seeds = [USER_SEED, user.key().as_ref(), mint.key().as_ref()],
        bump,
    )]
    pub user_account: Account<'info, UserAccount>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub mint: Account<'info, Mint>,

    /// User's external token wallet (source of deposit).
    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == mint.key(),
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// Program-controlled vault token account.
    #[account(
        mut,
        constraint = vault_token_account.mint == mint.key(),
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Settle<'info> {
    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump,
        has_one = authority,
    )]
    pub config: Account<'info, Config>,

    /// Only the authorized backend signer can settle.
    pub authority: Signer<'info>,

    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [USER_SEED, buyer_account.owner.as_ref(), mint.key().as_ref()],
        bump = buyer_account.bump,
    )]
    pub buyer_account: Account<'info, UserAccount>,

    #[account(
        mut,
        seeds = [USER_SEED, seller_account.owner.as_ref(), mint.key().as_ref()],
        bump = seller_account.bump,
    )]
    pub seller_account: Account<'info, UserAccount>,

    /// Fee recipient's user account (must match config.fee_recipient).
    #[account(
        mut,
        constraint = fee_account.owner == config.fee_recipient @ SettlementError::InvalidFeeRecipient,
        seeds = [USER_SEED, fee_account.owner.as_ref(), mint.key().as_ref()],
        bump = fee_account.bump,
    )]
    pub fee_account: Account<'info, UserAccount>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [USER_SEED, user.key().as_ref(), mint.key().as_ref()],
        bump = user_account.bump,
        constraint = user_account.owner == user.key(),
    )]
    pub user_account: Account<'info, UserAccount>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub mint: Account<'info, Mint>,

    /// User's external token wallet (destination of withdrawal).
    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == mint.key(),
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// Program-controlled vault token account.
    #[account(
        mut,
        constraint = vault_token_account.mint == mint.key(),
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    /// PDA that owns the vault token accounts.
    /// CHECK: derived from vault seeds, used as transfer authority.
    #[account(
        seeds = [VAULT_SEED, config.key().as_ref()],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct UpdateAuthority<'info> {
    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump,
        has_one = authority,
    )]
    pub config: Account<'info, Config>,

    pub authority: Signer<'info>,
}

// ── Events ───────────────────────────────────────────────

#[event]
pub struct ConfigInitialized {
    pub authority: Pubkey,
    pub fee_bps: u16,
}

#[event]
pub struct DepositEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub new_balance: u64,
}

#[event]
pub struct SettlementEvent {
    /// Platform fill ID (UUID bytes) linking to off-chain records.
    pub fill_id: [u8; 16],
    pub buyer: Pubkey,
    pub seller: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub fee: u64,
    pub seller_receives: u64,
}

#[event]
pub struct WithdrawEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub remaining_balance: u64,
}

#[event]
pub struct AuthorityUpdated {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}

#[event]
pub struct FeeUpdated {
    pub fee_bps: u16,
}

// ── Errors ───────────────────────────────────────────────

#[error_code]
pub enum SettlementError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Insufficient balance for this operation")]
    InsufficientBalance,

    #[msg("Arithmetic overflow")]
    Overflow,

    #[msg("Fee exceeds maximum allowed (50%)")]
    FeeTooHigh,

    #[msg("Fee account owner does not match config fee recipient")]
    InvalidFeeRecipient,
}
