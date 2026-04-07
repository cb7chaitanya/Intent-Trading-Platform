# IntentX Security Audit Report

**Date**: 2026-04-07
**Scope**: Solidity contracts, Anchor programs, Rust backend (auth, wallet, cross-chain)
**Severity scale**: Critical > High > Medium > Low

---

## Executive Summary

**51 vulnerabilities** identified across 3 layers. **8 Critical**, **13 High**, **22 Medium**, **8 Low**.

The most severe finding is the lack of on-chain signature verification in the Solidity settlement contract — a compromised authority key drains the entire vault in one transaction. The Solana programs have PDA validation gaps and no fill-ID deduplication. The backend has cross-chain replay risks, JWT algorithm confusion, CSRF bypass, and timing-attack-vulnerable key comparison.

**Recommendation: Do not deploy to mainnet without fixing all Critical and High findings.**

---

## Critical Findings (8)

### C-1: Solidity — No signature verification on settle()

**File**: `contracts/src/IntentXSettlement.sol`
**Impact**: Complete fund drainage

The `settle()` function accepts arbitrary buyer/seller/amount with only `onlyAuthority` protection. No EIP-712 signature from either party. A compromised authority key settles `victim_balance` to the attacker in one tx.

```
Attack: authority key leaked → settle(victim, attacker, token, MAX, fillId) → vault drained
```

**Fix**: Require EIP-712 signed settlement authorization from both buyer and seller. Add `mapping(bytes16 => bool) usedFillIds` to prevent replay.

---

### C-2: Solidity — Instant authority takeover, no timelock

**File**: `contracts/src/IntentXSettlement.sol:164-168`
**Impact**: Permanent lockout + fund theft

`updateAuthority()` takes effect in the same transaction. Compromised authority changes to attacker address, locks out legitimate authority, drains vault.

**Fix**: Two-step transfer with 48h timelock: `transferAuthority()` → waiting period → `acceptAuthority()`.

---

### C-3: Solana Settlement — Arbitrary buyer/seller account injection

**File**: `programs/intentx-settlement/src/lib.rs` (Settle context)
**Impact**: Fake settlement, fund theft

The `Settle` instruction derives buyer/seller PDAs from the accounts themselves (`buyer_account.owner`), but doesn't validate these match intended participants. Attacker creates fake user accounts with inflated balances, passes them to settle.

**Fix**: Add `buyer: Pubkey, seller: Pubkey` as explicit Settle parameters with constraints enforcing `buyer_account.owner == buyer`.

---

### C-4: Solana Settlement — No fill_id deduplication

**File**: `programs/intentx-settlement/src/lib.rs`
**Impact**: Double settlement, double-counting

`fill_id` is a parameter with no on-chain uniqueness check. Same fill_id can be submitted repeatedly, debiting buyer and crediting seller each time.

**Fix**: Add a PDA account derived from fill_id (`seeds = [b"fill", fill_id.as_ref()]`) that is initialized on first settle and prevents re-settlement.

---

### C-5: Backend — Unsigned transaction data injection

**File**: `src/wallet/ethereum.rs:415-422`
**Impact**: Sign arbitrary transactions

`sign_transaction()` deserializes `UnsignedTx.data` via `serde_json::from_slice()` without verifying authenticity. If an attacker controls this field, they can change the target address, amount, or calldata before signing.

**Fix**: HMAC the unsigned tx data at construction time. Verify HMAC before deserialization in `sign_transaction()`.

---

### C-6: Backend — Cross-chain replay (Ethereum)

**File**: `src/wallet/eth_sign.rs:109-146`
**Impact**: Transaction replayed on wrong chain

`sign_legacy_tx()` and `sign_eip1559_tx()` accept `chain_id` as parameter but don't validate it matches the intended network. A signature for Sepolia can be broadcast on mainnet if chain_id is incorrectly passed.

**Fix**: Validate `chain_id` against a configured allowlist per environment. Reject signing if chain_id doesn't match the expected deployment.

---

### C-7: Backend — Solana transaction replay

**File**: `src/wallet/solana_signing.rs:117-140`
**Impact**: Transaction replayed multiple times

Ed25519 signatures don't include chain-specific metadata. The signing functions don't enforce that `recent_blockhash` or sequence numbers are included in the message. Same signed message can be submitted multiple times.

**Fix**: Document and enforce at the call site that `recent_blockhash` must be included. Add validation that message contains a blockhash before signing.

---

### C-8: Backend — JWT algorithm confusion

**File**: `src/auth/jwt.rs:123-129`
**Impact**: Forge any JWT, impersonate any user

`Header::default()` doesn't enforce algorithm. If an attacker sends `{"alg":"none"}`, the token may pass validation without signature verification.

**Fix**: Explicitly set `Header { alg: Algorithm::HS256, .. }` on encode. Set `Validation::new(Algorithm::HS256)` with `validate_exp = true` on decode.

---

## High Findings (13)

### H-1: Solidity — Admin functions bypass pause
`updateFee()`, `updateAuthority()`, `updateFeeRecipient()` all execute while paused. Attacker changes parameters during incident response.
**Fix**: Apply `whenNotPaused` to admin functions or add separate admin pause.

### H-2: Solidity — Fee rounding down leakage
`(amount * feeBps) / 10_000` rounds down. Millions of micro-settlements leak protocol revenue.
**Fix**: Round up on fees: `(amount * feeBps + 9999) / 10_000`.

### H-3: Solidity — No slippage/price validation in settle()
No on-chain check that settlement amount is fair. MEV attacker front-runs `updateFee()` to change fee before settlement executes.
**Fix**: Include fee snapshot in settlement parameters, validate on-chain.

### H-4: Solana Settlement — Vault token account ownership not enforced
`vault_token_account` constraint checks mint but not ownership. Attacker substitutes their own account as the vault.
**Fix**: Add `constraint = vault_token_account.owner == vault_authority.key()`.

### H-5: Solana Settlement — Unchecked fee_recipient
`initialize()` accepts any pubkey as `fee_recipient` without validating it's a real participant.
**Fix**: Require `fee_recipient` to be a `Signer` in initialize.

### H-6: Backend — CSRF token fixation
CSRF middleware validates header token but optionally matches cookie. If no cookie present, header-only token bypasses double-submit check.
**Fix**: Require BOTH header AND cookie tokens present and matching.

### H-7: Backend — Timing attack on API key hash comparison
`k.key_hash == hash` is not constant-time. Attacker measures response time to narrow key space.
**Fix**: Use `subtle::ConstantTimeEq` for hash comparison.

### H-8: Backend — JWT weak secret fallback
If key rotation service isn't initialized, falls back to config secret (likely hardcoded/weak).
**Fix**: Fail explicitly if key rotation unavailable. Never fall back to static secret.

### H-9: Backend — Gas price manipulation via RPC
`base_fee * BASE_FEE_MULTIPLIER` can overflow if RPC returns inflated base_fee. Drains user's gas budget.
**Fix**: Add global `MAX_FEE_CAP` and use `checked_mul()`.

### H-10: Solana HTLC — Front-running claim via mempool
Anyone can submit `claim()` with the secret. Attacker sees secret in pending tx and front-runs.
**Fix**: Document as accepted behavior (tokens always go to designated receiver) OR add receiver-must-sign requirement.

### H-11: Backend — Signature malleability (Ethereum)
ECDSA signatures can have high/low S values. No normalization to low-S form.
**Fix**: Call `normalize_s()` after signing to enforce canonical signatures.

### H-12: Solidity — Arbitrary fee recipient control
Authority changes feeRecipient instantly. All future fees route to attacker wallet.
**Fix**: Add 24h timelock on feeRecipient changes.

### H-13: Backend — Missing JWT expiry validation
`validate_token_sync` uses `Validation::default()` which may not enforce `exp` claim.
**Fix**: Explicitly set `validation.validate_exp = true`.

---

## Medium Findings (22)

| ID | Component | Finding |
|----|-----------|---------|
| M-1 | Solidity | No emergency fund recovery for accidentally-sent tokens |
| M-2 | Solidity | Approval race condition on deposit (ERC-20 approve front-run) |
| M-3 | Solana HTLC | Timelock can be set to i64::MAX, permanently locking funds |
| M-4 | Solana HTLC | Missing rent exemption validation on escrow |
| M-5 | Solana Settlement | Authority update has no timelock |
| M-6 | Solana Settlement | Fee account must pre-exist or settle fails |
| M-7 | Solana Settlement | total_volume overflow DOSes settle at u64::MAX |
| M-8 | Backend signing | Panic in AES-GCM encryption leaks implementation details |
| M-9 | Backend Solana | Base58 decoder unbounded — memory exhaustion DoS |
| M-10 | Backend Ethereum | Nonce staleness between fetch and sign |
| M-11 | Backend JWT | Missing audience (aud) claim — cross-service token reuse |
| M-12 | Backend auth | No Bearer token format validation |
| M-13 | Backend CSRF | Bypass via GET if endpoints accept state changes on GET |
| M-14 | Backend CSRF | Non-atomic Redis GET+DEL allows token reuse in race |
| M-15 | Backend API keys | Weak key generation (128 bits, should be 256) |
| M-16 | Backend API keys | 8-char prefix enables enumeration (only 2^32 values) |
| M-17 | Backend signing | Nonce reuse across different secrets |
| M-18 | Backend verifier | 30s timestamp window too loose for replay |
| M-19 | Backend verifier | Nonce TTL (60s) > request window (30s) — replay gap |
| M-20 | Backend verifier | No request body canonicalization |
| M-21 | Backend gateway | SSRF via email header injection |
| M-22 | Backend gateway | Header injection via permission list newlines |

---

## Low Findings (8)

| ID | Component | Finding |
|----|-----------|---------|
| L-1 | Solidity | Buyer and seller can be same address (self-settlement) |
| L-2 | Solidity | fillId collision allowed (no uniqueness tracking) |
| L-3 | Solidity | No token whitelist (malicious token can DoS vault) |
| L-4 | Solidity | Pause/unpause callable when already in target state |
| L-5 | Solana HTLC | FundsClaimed event reveals secret (by design but should document) |
| L-6 | Solana HTLC | Receiver can be a PDA (funds stuck if PDA can't sign) |
| L-7 | Backend signing | No timing attack protection on key comparison (low impact) |
| L-8 | Backend gateway | API key service unavailable returns 500 instead of 401 |

---

## Cross-Chain Specific Findings

### XC-1: Bridge worker crash between lock_funds() and DB update
**Severity**: High (operational)
Funds locked on-chain but leg stays "pending" in DB. Worker re-processes on restart and may double-lock.
**Fix**: Make lock_funds idempotent by checking on-chain state before re-locking.

### XC-2: Cascading refund not atomic
**Severity**: High
`process_timeouts()` refunds source leg, then dest leg in separate SQL calls. Crash between them leaves dest leg orphaned.
**Fix**: Wrap both refunds in a single SQL transaction.

### XC-3: Leg status transitions have no previous-status guard
**Severity**: High
`update_leg_status()` uses `WHERE id = $1` without `AND status = $expected`. Concurrent workers can race and overwrite.
**Fix**: Add `AND status = $previous_status` to all leg transitions.

### XC-4: Intent finalization race
**Severity**: Medium
`finalize_completed()` reads both legs, then updates intent status. Between read and write, legs could fail.
**Fix**: Use subquery in UPDATE: `WHERE id = $1 AND EXISTS (both legs confirmed)`.

### XC-5: HTLC claim vs refund race on expired timelock
**Severity**: Medium
Both claim and refund can target `status = 'source_locked'`. SQL WHERE clause provides mutual exclusion, but no DB-level constraint prevents both paths from being attempted simultaneously.
**Fix**: Property test confirms only one wins (tested in invariant_proptest.rs). Add explicit CHECK constraint or use SELECT FOR UPDATE.

### XC-6: balance transfer() not atomic
**Severity**: Critical (backend)
`balances/service.rs` transfer() makes two separate UPDATE calls. Crash between them loses funds.
**Fix**: Wrap in explicit SQL transaction.

---

## MEV / Solver Manipulation Vectors

### MEV-1: Solver bid sniping
Solver observes another solver's winning bid in the auction, submits a slightly better bid at the last moment. No commitment scheme prevents this.
**Fix**: Sealed-bid auction with commit-reveal: solvers commit hash(bid) first, reveal after deadline.

### MEV-2: Settlement front-running
Authority's `settle()` tx is visible in mempool. MEV bot sandwiches the settlement to extract value from price movement.
**Fix**: Use private mempool (Flashbots Protect) for settlement transactions.

### MEV-3: Oracle price manipulation
Oracle price feeds (`/oracle/prices`) influence TWAP and stop orders. If oracle source is a single DEX, flash-loan attacks can manipulate price to trigger stops.
**Fix**: Use TWAP oracle with multiple sources, minimum observation window.

### MEV-4: Cross-chain arbitrage via delayed VAA
Solver sees VAA before submitting to dest chain, uses the delay to arbitrage price differences between chains.
**Fix**: Time-bound settlements with minimum execution speed requirements.

---

## Test Coverage Gaps

| Area | Current Coverage | Missing |
|------|-----------------|---------|
| Solidity settle() | 77% | Authority compromise, duplicate fillId, self-settlement, MEV |
| Solana HTLC | 30% (structural only) | On-chain claim/refund, timelock boundary, double-claim, wrong secret |
| Solana Settlement | 15% (constants only) | Settle authorization, double settlement, vault ownership, deposit/withdraw |
| Backend JWT | Not tested | Algorithm confusion, expiry validation, audience claim |
| Backend CSRF | Not tested | Token fixation, GET bypass, race condition |
| Backend signing | Unit tests only | Cross-chain replay, malleability, gas manipulation |

---

## Recommended Fix Priority

### Week 1 (Critical)
1. Add EIP-712 signature verification to Solidity settle()
2. Add fillId deduplication to both Solidity and Solana contracts
3. Add two-step authority transfer with timelock
4. Fix JWT algorithm enforcement
5. Fix unsigned tx data injection (HMAC)
6. Fix balance transfer() atomicity

### Week 2 (High)
7. Add vault ownership constraints in Solana Settlement
8. Add fee_recipient validation in Solana Settlement
9. Fix CSRF double-submit enforcement
10. Add constant-time API key comparison
11. Fix admin function pause bypass
12. Add chain_id validation in Ethereum signing

### Week 3 (Medium + Tests)
13. Add property-based fuzz tests for all settlement paths
14. Add adversarial authority tests
15. Add cross-chain race condition tests
16. Fix all Medium findings
17. Add Solana behavioral tests (not just structural)

---

## Appendix: Verified Mitigations

These security measures ARE correctly implemented:

- Solidity ReentrancyGuard on deposit/withdraw/settle
- Solidity Pausable mechanism (user operations)
- Solidity Solidity 0.8.24 overflow protection
- Solana Anchor PDA derivation with seed validation
- Solana HTLC SHA-256 secret-hash verification
- Solana HTLC timelock enforcement (`Clock::get()` comparison)
- Backend AES-256-GCM key encryption
- Backend Keccak-256 address derivation
- Backend EIP-155 chain_id in legacy tx v value
- Backend circuit breakers on all external calls
- Backend exponential backoff with jitter
