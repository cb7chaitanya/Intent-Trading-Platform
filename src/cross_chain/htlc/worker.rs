//! HTLC swap worker — drives the atomic swap lifecycle.
//!
//! Six-phase poll cycle (every 5 seconds):
//! 1. Lock source: Created swaps → bridge.lock_funds → SourceLocked
//! 2. Monitor dest lock: SourceLocked + no dest_lock_tx → check solver
//! 3. Claim destination: SourceLocked + dest_lock_tx → reveal secret
//! 4. Unlock source: DestClaimed + secret revealed → complete swap
//! 5. Refund expired: past timelock + not claimed → refund
//! 6. Metrics update

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_chain::bridge::{BridgeStatus, BridgeTransferParams};
use crate::cross_chain::bridge_registry::BridgeRegistry;
use crate::metrics::{counters, histograms};

use super::crypto;
use super::service::HtlcService;

const POLL_INTERVAL_SECS: u64 = 5;

// ── Entry point ──────────────────────────────────────────

pub async fn run(
    htlc: Arc<HtlcService>,
    bridges: Arc<BridgeRegistry>,
    cancel: CancellationToken,
) {
    tracing::info!(
        poll_secs = POLL_INTERVAL_SECS,
        "HTLC swap worker started"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                let start = std::time::Instant::now();

                let locked = phase_lock_source(&htlc, &bridges).await;
                let dest_locked = phase_monitor_dest_lock(&htlc, &bridges).await;
                let claimed = phase_claim_destination(&htlc, &bridges).await;
                let unlocked = phase_unlock_source(&htlc, &bridges).await;
                let refunded = phase_refund_expired(&htlc).await;

                let ms = start.elapsed().as_millis();
                if locked + dest_locked + claimed + unlocked + refunded > 0 {
                    tracing::info!(
                        locked, dest_locked, claimed, unlocked, refunded, ms,
                        "htlc_worker_cycle"
                    );
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("HTLC swap worker shutting down");
                return;
            }
        }
    }
}

// ── Phase 1: Lock funds on source chain ──────────────────

#[tracing::instrument(skip_all, name = "htlc.lock_source")]
async fn phase_lock_source(htlc: &HtlcService, bridges: &BridgeRegistry) -> u32 {
    let swaps = match htlc.find_pending_locks().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "htlc_pending_locks_query_failed");
            return 0;
        }
    };

    let mut n = 0u32;
    for swap in &swaps {
        let bridge = match bridges.find(&swap.source_chain, &swap.dest_chain) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(htlc_id = %swap.id, error = %e, "htlc_no_bridge");
                let _ = htlc.fail_swap(swap.id, &e.to_string()).await;
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
                continue;
            }
        };

        let params = BridgeTransferParams {
            source_chain: swap.source_chain.clone(),
            dest_chain: swap.dest_chain.clone(),
            token: swap.source_token.clone().unwrap_or_default(),
            amount: swap.source_amount as u64,
            sender: swap.source_sender.clone(),
            recipient: swap.source_receiver.clone(),
        };

        match bridge.lock_funds(&params).await {
            Ok(receipt) => {
                if let Err(e) = htlc.record_source_lock(swap.id, &receipt.tx_hash).await {
                    tracing::error!(htlc_id = %swap.id, error = %e, "htlc_record_lock_failed");
                    continue;
                }
                tracing::info!(
                    htlc_id = %swap.id,
                    bridge = bridge.name(),
                    tx = %receipt.tx_hash,
                    "htlc_source_locked"
                );
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["started"]).inc();
                n += 1;
            }
            Err(e) => {
                tracing::error!(htlc_id = %swap.id, error = %e, "htlc_source_lock_failed");
                let _ = htlc.fail_swap(swap.id, &format!("Source lock failed: {e}")).await;
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
            }
        }
    }
    n
}

// ── Phase 2: Monitor solver's destination lock ───────────

async fn phase_monitor_dest_lock(htlc: &HtlcService, bridges: &BridgeRegistry) -> u32 {
    let swaps = match htlc.find_awaiting_dest_lock().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "htlc_awaiting_dest_query_failed");
            return 0;
        }
    };

    let mut n = 0u32;
    for swap in &swaps {
        // Check if the solver has locked on the destination chain.
        // In production, this would query the dest chain HTLC contract
        // or watch for events. For now, we use bridge.verify_lock on
        // the source tx to see if the bridge message was delivered.
        let source_tx = match &swap.source_lock_tx {
            Some(tx) => tx.as_str(),
            None => continue,
        };

        let bridge = match bridges.find(&swap.source_chain, &swap.dest_chain) {
            Ok(b) => b,
            Err(_) => continue,
        };

        match bridge.verify_lock(source_tx).await {
            Ok(BridgeStatus::Completed { dest_tx_hash }) => {
                // Bridge delivered — treat as dest lock
                if let Err(e) = htlc.record_dest_lock(swap.id, &dest_tx_hash).await {
                    tracing::warn!(htlc_id = %swap.id, error = %e, "htlc_record_dest_lock_failed");
                } else {
                    tracing::info!(htlc_id = %swap.id, dest_tx = %dest_tx_hash, "htlc_dest_locked");
                    n += 1;
                }
            }
            Ok(BridgeStatus::InTransit { message_id }) => {
                // Message in transit — solver should lock soon
                tracing::debug!(htlc_id = %swap.id, message_id, "htlc_bridge_in_transit");
            }
            Ok(BridgeStatus::Failed { reason }) => {
                let _ = htlc.fail_swap(swap.id, &format!("Bridge failed: {reason}")).await;
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
            }
            _ => {}
        }
    }
    n
}

// ── Phase 3: Claim destination HTLC (reveal secret) ─────

async fn phase_claim_destination(htlc: &HtlcService, bridges: &BridgeRegistry) -> u32 {
    let swaps = match htlc.find_claimable().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "htlc_claimable_query_failed");
            return 0;
        }
    };

    let mut n = 0u32;
    for swap in &swaps {
        // Retrieve the secret from the DB. It must have been stored
        // via store_secret() right after create_swap().
        let secret: crypto::Secret = match &swap.secret {
            Some(s) if s.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(s);
                arr
            }
            Some(s) => {
                tracing::error!(
                    htlc_id = %swap.id,
                    len = s.len(),
                    "htlc_stored_secret_wrong_length"
                );
                let _ = htlc.fail_swap(swap.id, "Stored secret has wrong length").await;
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
                continue;
            }
            None => {
                tracing::warn!(
                    htlc_id = %swap.id,
                    "htlc_no_secret_stored_skipping_claim"
                );
                continue;
            }
        };

        // Verify the secret matches the hash before using it on-chain
        let expected: crypto::SecretHash = match swap.secret_hash.clone().try_into() {
            Ok(h) => h,
            Err(_) => {
                tracing::error!(htlc_id = %swap.id, "htlc_hash_wrong_length");
                let _ = htlc.fail_swap(swap.id, "Secret hash wrong length").await;
                continue;
            }
        };

        if !crypto::verify_secret(&secret, &expected) {
            tracing::error!(
                htlc_id = %swap.id,
                "htlc_secret_does_not_match_hash"
            );
            let _ = htlc.fail_swap(swap.id, "Stored secret does not match hash").await;
            counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
            continue;
        }

        let bridge = match bridges.find(&swap.source_chain, &swap.dest_chain) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let params = BridgeTransferParams {
            source_chain: swap.source_chain.clone(),
            dest_chain: swap.dest_chain.clone(),
            token: swap.dest_token.clone().unwrap_or_default(),
            amount: swap.dest_amount as u64,
            sender: swap.dest_sender.clone(),
            recipient: swap.dest_receiver.clone(),
        };

        // The message_id encodes the secret hex so the bridge adapter
        // can include it in the on-chain claim transaction calldata.
        let msg_id = format!("htlc_secret_{}", crypto::to_hex(&secret));

        match bridge.release_funds(&params, &msg_id).await {
            Ok(claim_tx) => {
                // Record the claim with the verified secret
                if let Err(e) = htlc
                    .record_dest_claim(swap.id, &secret, &claim_tx)
                    .await
                {
                    tracing::warn!(htlc_id = %swap.id, error = %e, "htlc_claim_record_failed");
                    continue;
                }

                tracing::info!(
                    htlc_id = %swap.id,
                    claim_tx = %claim_tx,
                    secret_hex = %crypto::to_hex(&secret),
                    "htlc_dest_claimed_with_secret"
                );
                n += 1;
            }
            Err(e) => {
                tracing::error!(htlc_id = %swap.id, error = %e, "htlc_dest_claim_failed");
            }
        }
    }
    n
}

// ── Phase 4: Unlock source chain with revealed secret ────

#[tracing::instrument(skip_all, name = "htlc.unlock_source")]
async fn phase_unlock_source(htlc: &HtlcService, bridges: &BridgeRegistry) -> u32 {
    let swaps = match htlc.find_pending_unlocks().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "htlc_pending_unlocks_query_failed");
            return 0;
        }
    };

    let mut n = 0u32;
    for swap in &swaps {
        let secret = match &swap.secret {
            Some(s) if s.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(s);
                arr
            }
            _ => {
                tracing::warn!(htlc_id = %swap.id, "htlc_missing_secret_for_unlock");
                continue;
            }
        };

        // Verify the secret against the stored hash one more time
        // before submitting to the source chain. This guards against
        // DB corruption or a buggy claim phase.
        let expected: crypto::SecretHash = match swap.secret_hash.clone().try_into() {
            Ok(h) => h,
            Err(_) => {
                tracing::error!(htlc_id = %swap.id, "htlc_hash_wrong_length_in_unlock");
                continue;
            }
        };

        if !crypto::verify_secret(&secret, &expected) {
            tracing::error!(
                htlc_id = %swap.id,
                "htlc_secret_hash_mismatch_in_unlock"
            );
            let _ = htlc.fail_swap(swap.id, "Secret/hash mismatch at unlock").await;
            counters::HTLC_SWAPS_TOTAL.with_label_values(&["failed"]).inc();
            continue;
        }

        let bridge = match bridges.find(&swap.source_chain, &swap.dest_chain) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Submit the secret to the source chain HTLC contract.
        // The message_id carries the secret hex so the bridge adapter
        // can build the claim(secret) calldata.
        let params = BridgeTransferParams {
            source_chain: swap.source_chain.clone(),
            dest_chain: swap.dest_chain.clone(),
            token: swap.source_token.clone().unwrap_or_default(),
            amount: swap.source_amount as u64,
            sender: swap.source_sender.clone(),
            recipient: swap.source_receiver.clone(),
        };

        let msg = format!("htlc_unlock_{}", crypto::to_hex(&secret));

        match bridge.release_funds(&params, &msg).await {
            Ok(unlock_tx) => {
                if let Err(e) = htlc.complete_swap(swap.id, &unlock_tx).await {
                    tracing::error!(htlc_id = %swap.id, error = %e, "htlc_complete_failed");
                    continue;
                }

                let duration = (chrono::Utc::now() - swap.created_at).num_seconds().max(0) as f64;
                histograms::HTLC_SWAP_DURATION.observe(duration);
                counters::HTLC_SWAPS_TOTAL.with_label_values(&["completed"]).inc();

                tracing::info!(
                    htlc_id = %swap.id,
                    unlock_tx = %unlock_tx,
                    duration_secs = duration,
                    "htlc_swap_completed"
                );
                n += 1;
            }
            Err(e) => {
                tracing::error!(htlc_id = %swap.id, error = %e, "htlc_source_unlock_failed");
            }
        }
    }
    n
}

// ── Phase 5: Refund expired swaps ────────────────────────

#[tracing::instrument(skip_all, name = "htlc.refund_expired")]
async fn phase_refund_expired(htlc: &HtlcService) -> u32 {
    let swaps = match htlc.find_expired().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "htlc_expired_query_failed");
            return 0;
        }
    };

    let mut n = 0u32;
    for swap in &swaps {
        if let Err(e) = htlc.refund_swap(swap.id).await {
            tracing::error!(htlc_id = %swap.id, error = %e, "htlc_refund_failed");
            continue;
        }

        let duration = (chrono::Utc::now() - swap.created_at).num_seconds().max(0) as f64;
        histograms::HTLC_SWAP_DURATION.observe(duration);
        counters::HTLC_SWAPS_TOTAL.with_label_values(&["refunded"]).inc();

        tracing::warn!(
            htlc_id = %swap.id,
            source_chain = %swap.source_chain,
            dest_chain = %swap.dest_chain,
            amount = swap.source_amount,
            duration_secs = duration,
            "htlc_swap_refunded"
        );
        n += 1;
    }
    n
}
