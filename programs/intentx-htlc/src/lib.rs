use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("HtLc1111111111111111111111111111111111111111");

// ── Seeds ────────────────────────────────────────────────

pub const HTLC_SEED: &[u8] = b"htlc";
pub const ESCROW_SEED: &[u8] = b"escrow";

// ── Program ──────────────────────────────────────────────

#[program]
pub mod intentx_htlc {
    use super::*;

    /// Create and fund an HTLC in a single transaction.
    ///
    /// The sender locks `amount` tokens into a PDA-controlled escrow.
    /// The receiver can claim by revealing the preimage of `hashlock`.
    /// If unclaimed after `timelock` (unix timestamp), the sender can refund.
    pub fn lock_funds(
        ctx: Context<LockFunds>,
        hashlock: [u8; 32],
        timelock: i64,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, HtlcError::ZeroAmount);
        let now = Clock::get()?.unix_timestamp;
        require!(timelock > now, HtlcError::TimelockInPast);

        // Transfer tokens: sender → escrow
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.sender_token_account.to_account_info(),
                to: ctx.accounts.escrow_token_account.to_account_info(),
                authority: ctx.accounts.sender.to_account_info(),
            },
        );
        token::transfer(cpi_ctx, amount)?;

        // Initialize HTLC state
        let sender_key = ctx.accounts.sender.key();
        let receiver_key = ctx.accounts.receiver.key();
        let mint_key = ctx.accounts.mint.key();
        let htlc_key = ctx.accounts.htlc.key();

        let htlc = &mut ctx.accounts.htlc;
        htlc.sender = sender_key;
        htlc.receiver = receiver_key;
        htlc.mint = mint_key;
        htlc.hashlock = hashlock;
        htlc.timelock = timelock;
        htlc.amount = amount;
        htlc.claimed = false;
        htlc.refunded = false;
        htlc.bump = ctx.bumps.htlc;
        htlc.escrow_bump = ctx.bumps.escrow_token_account;

        emit!(FundsLocked {
            htlc: htlc_key,
            sender: sender_key,
            receiver: receiver_key,
            mint: mint_key,
            hashlock,
            timelock,
            amount,
        });

        Ok(())
    }

    /// Claim funds by revealing the secret (preimage of hashlock).
    ///
    /// Anyone can submit the claim, but tokens always go to the
    /// designated receiver. Verifies SHA-256(secret) == hashlock.
    pub fn claim(ctx: Context<Claim>, secret: [u8; 32]) -> Result<()> {
        // Copy all needed values before mutating
        let hashlock = ctx.accounts.htlc.hashlock;
        let escrow_bump = ctx.accounts.htlc.escrow_bump;
        let amount = ctx.accounts.htlc.amount;
        let receiver = ctx.accounts.htlc.receiver;
        let htlc_key = ctx.accounts.htlc.key();

        require!(!ctx.accounts.htlc.claimed, HtlcError::AlreadyClaimed);
        require!(!ctx.accounts.htlc.refunded, HtlcError::AlreadyRefunded);

        // Verify preimage
        let hash = anchor_lang::solana_program::hash::hash(&secret);
        require!(hash.to_bytes() == hashlock, HtlcError::InvalidSecret);

        // Transfer tokens: escrow → receiver (PDA-signed)
        let bump_bytes = [escrow_bump];
        let signer_seeds: &[&[u8]] = &[ESCROW_SEED, hashlock.as_ref(), &bump_bytes];
        let signer = &[signer_seeds][..];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.receiver_token_account.to_account_info(),
                authority: ctx.accounts.escrow_token_account.to_account_info(),
            },
            signer,
        );
        token::transfer(cpi_ctx, amount)?;

        // Mark claimed
        ctx.accounts.htlc.claimed = true;

        emit!(FundsClaimed {
            htlc: htlc_key,
            receiver,
            secret,
            amount,
        });

        Ok(())
    }

    /// Refund funds to sender after the timelock has expired.
    ///
    /// Only possible if the HTLC has not been claimed.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let hashlock = ctx.accounts.htlc.hashlock;
        let escrow_bump = ctx.accounts.htlc.escrow_bump;
        let amount = ctx.accounts.htlc.amount;
        let sender = ctx.accounts.htlc.sender;
        let htlc_key = ctx.accounts.htlc.key();

        require!(!ctx.accounts.htlc.claimed, HtlcError::AlreadyClaimed);
        require!(!ctx.accounts.htlc.refunded, HtlcError::AlreadyRefunded);

        let now = Clock::get()?.unix_timestamp;
        require!(now >= ctx.accounts.htlc.timelock, HtlcError::TimelockNotExpired);

        // Transfer tokens: escrow → sender (PDA-signed)
        let bump_bytes = [escrow_bump];
        let signer_seeds: &[&[u8]] = &[ESCROW_SEED, hashlock.as_ref(), &bump_bytes];
        let signer = &[signer_seeds][..];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.sender_token_account.to_account_info(),
                authority: ctx.accounts.escrow_token_account.to_account_info(),
            },
            signer,
        );
        token::transfer(cpi_ctx, amount)?;

        ctx.accounts.htlc.refunded = true;

        emit!(FundsRefunded {
            htlc: htlc_key,
            sender,
            amount,
        });

        Ok(())
    }
}

// ── State ────────────────────────────────────────────────

#[account]
pub struct Htlc {
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub mint: Pubkey,
    /// SHA-256(secret). The lock condition.
    pub hashlock: [u8; 32],
    /// Unix timestamp after which sender can refund.
    pub timelock: i64,
    /// Amount of tokens locked.
    pub amount: u64,
    pub claimed: bool,
    pub refunded: bool,
    /// PDA bump for the HTLC account.
    pub bump: u8,
    /// PDA bump for the escrow token account.
    pub escrow_bump: u8,
}

impl Htlc {
    pub const SIZE: usize = 8  // discriminator
        + 32  // sender
        + 32  // receiver
        + 32  // mint
        + 32  // hashlock
        + 8   // timelock
        + 8   // amount
        + 1   // claimed
        + 1   // refunded
        + 1   // bump
        + 1;  // escrow_bump
}

// ── Instruction Contexts ─────────────────────────────────

#[derive(Accounts)]
#[instruction(hashlock: [u8; 32], timelock: i64, amount: u64)]
pub struct LockFunds<'info> {
    /// HTLC state account, PDA derived from hashlock.
    #[account(
        init,
        payer = sender,
        space = Htlc::SIZE,
        seeds = [HTLC_SEED, hashlock.as_ref()],
        bump,
    )]
    pub htlc: Account<'info, Htlc>,

    /// Escrow token account, PDA that holds the locked tokens.
    /// The escrow is its own authority (self-referential PDA).
    #[account(
        init,
        payer = sender,
        token::mint = mint,
        token::authority = escrow_token_account,
        seeds = [ESCROW_SEED, hashlock.as_ref()],
        bump,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Sender's wallet (pays for accounts + locks tokens).
    #[account(mut)]
    pub sender: Signer<'info>,

    /// The intended receiver (stored in HTLC state, not a signer).
    /// CHECK: stored as recipient, validated at claim time.
    pub receiver: UncheckedAccount<'info>,

    pub mint: Account<'info, Mint>,

    /// Sender's token account (source of locked tokens).
    #[account(
        mut,
        constraint = sender_token_account.owner == sender.key(),
        constraint = sender_token_account.mint == mint.key(),
    )]
    pub sender_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(
        mut,
        seeds = [HTLC_SEED, htlc.hashlock.as_ref()],
        bump = htlc.bump,
        constraint = !htlc.claimed @ HtlcError::AlreadyClaimed,
        constraint = !htlc.refunded @ HtlcError::AlreadyRefunded,
    )]
    pub htlc: Account<'info, Htlc>,

    /// Escrow holding the locked tokens.
    #[account(
        mut,
        seeds = [ESCROW_SEED, htlc.hashlock.as_ref()],
        bump = htlc.escrow_bump,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Receiver's token account (destination).
    #[account(
        mut,
        constraint = receiver_token_account.owner == htlc.receiver @ HtlcError::WrongReceiver,
        constraint = receiver_token_account.mint == htlc.mint,
    )]
    pub receiver_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(
        mut,
        seeds = [HTLC_SEED, htlc.hashlock.as_ref()],
        bump = htlc.bump,
        has_one = sender,
        constraint = !htlc.claimed @ HtlcError::AlreadyClaimed,
        constraint = !htlc.refunded @ HtlcError::AlreadyRefunded,
    )]
    pub htlc: Account<'info, Htlc>,

    /// Only the original sender can refund.
    pub sender: Signer<'info>,

    /// Escrow holding the locked tokens.
    #[account(
        mut,
        seeds = [ESCROW_SEED, htlc.hashlock.as_ref()],
        bump = htlc.escrow_bump,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// Sender's token account (refund destination).
    #[account(
        mut,
        constraint = sender_token_account.owner == sender.key(),
        constraint = sender_token_account.mint == htlc.mint,
    )]
    pub sender_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// ── Events ───────────────────────────────────────────────

#[event]
pub struct FundsLocked {
    pub htlc: Pubkey,
    pub sender: Pubkey,
    pub receiver: Pubkey,
    pub mint: Pubkey,
    pub hashlock: [u8; 32],
    pub timelock: i64,
    pub amount: u64,
}

#[event]
pub struct FundsClaimed {
    pub htlc: Pubkey,
    pub receiver: Pubkey,
    pub secret: [u8; 32],
    pub amount: u64,
}

#[event]
pub struct FundsRefunded {
    pub htlc: Pubkey,
    pub sender: Pubkey,
    pub amount: u64,
}

// ── Errors ───────────────────────────────────────────────

#[error_code]
pub enum HtlcError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Timelock must be in the future")]
    TimelockInPast,
    #[msg("Invalid secret: hash does not match")]
    InvalidSecret,
    #[msg("HTLC already claimed")]
    AlreadyClaimed,
    #[msg("HTLC already refunded")]
    AlreadyRefunded,
    #[msg("Timelock has not expired yet")]
    TimelockNotExpired,
    #[msg("Receiver token account does not match HTLC receiver")]
    WrongReceiver,
}
