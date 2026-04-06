//! Unified multi-chain transaction confirmation worker.
//!
//! Polls all submitted transactions, routing each through the correct
//! chain adapter based on `tx.chain`. Handles chain-specific confirmation
//! thresholds and drop timeouts (Ethereum: 12 blocks / 10min drop,
//! Solana: 31 confirmations / 120s drop).

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::chain::TxState;
use super::model::{TransactionRecord, TxStatus};
use super::service::WalletService;

/// How often the worker polls for pending transactions.
const POLL_INTERVAL_SECS: u64 = 3;

/// Maximum transactions to process per poll cycle.
const BATCH_SIZE: usize = 100;

// ── Worker entry point ───────────────────────────────────

/// Start the confirmation worker. Runs until the cancellation token fires.
pub async fn run(wallet_service: Arc<WalletService>, cancel: CancellationToken) {
    let chains = wallet_service.chains().chains();
    tracing::info!(
        poll_secs = POLL_INTERVAL_SECS,
        chains = ?chains,
        "Confirmation worker started (multi-chain)"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                if let Err(e) = poll_cycle(&wallet_service).await {
                    tracing::error!(error = %e, "confirmation_poll_error");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Confirmation worker shutting down");
                return;
            }
        }
    }
}

// ── Poll cycle ───────────────────────────────────────────

async fn poll_cycle(svc: &WalletService) -> Result<(), String> {
    let pending = svc
        .get_pending_transactions()
        .await
        .map_err(|e| e.to_string())?;

    if pending.is_empty() {
        return Ok(());
    }

    let mut confirmed_count = 0u32;
    let mut failed_count = 0u32;
    let mut dropped_count = 0u32;

    for tx in pending.iter().take(BATCH_SIZE) {
        match process_tx(svc, tx).await {
            TxOutcome::Confirmed => confirmed_count += 1,
            TxOutcome::Failed => failed_count += 1,
            TxOutcome::Dropped => dropped_count += 1,
            TxOutcome::Pending | TxOutcome::Skipped => {}
        }
    }

    if confirmed_count + failed_count + dropped_count > 0 {
        tracing::info!(
            confirmed = confirmed_count,
            failed = failed_count,
            dropped = dropped_count,
            pending = pending.len(),
            "confirmation_poll_complete"
        );
    }

    Ok(())
}

// ── Per-transaction processing ───────────────────────────

enum TxOutcome {
    Confirmed,
    Failed,
    Dropped,
    Pending,
    Skipped,
}

async fn process_tx(svc: &WalletService, tx: &TransactionRecord) -> TxOutcome {
    // Only process submitted transactions
    if tx.status != TxStatus::Submitted {
        return TxOutcome::Skipped;
    }

    let tx_hash = match &tx.tx_hash {
        Some(h) => h.as_str(),
        None => return TxOutcome::Skipped,
    };

    // Route to the correct chain adapter
    let adapter = match svc.chains().get(&tx.chain) {
        Ok(a) => a,
        Err(_) => {
            tracing::warn!(
                tx_id = %tx.id,
                chain = %tx.chain,
                "no_adapter_for_chain"
            );
            return TxOutcome::Skipped;
        }
    };

    let required = adapter.required_confirmations();
    let drop_timeout = adapter.drop_timeout_secs();

    // Query chain for current status
    match adapter.get_transaction_status(tx_hash).await {
        Ok(TxState::Confirmed { block, confirmations }) => {
            handle_confirmed(svc, tx, tx_hash, block, confirmations, required).await
        }
        Ok(TxState::Failed { reason }) => {
            handle_failed(svc, tx, tx_hash, &reason).await
        }
        Ok(TxState::Pending) => {
            handle_pending(svc, tx, tx_hash, drop_timeout).await
        }
        Err(e) => {
            tracing::warn!(
                tx_id = %tx.id,
                chain = %tx.chain,
                tx_hash = %tx_hash,
                error = %e,
                "status_check_error"
            );
            TxOutcome::Pending
        }
    }
}

// ── State handlers ───────────────────────────────────────

async fn handle_confirmed(
    svc: &WalletService,
    tx: &TransactionRecord,
    tx_hash: &str,
    block: u64,
    confirmations: u32,
    required: u32,
) -> TxOutcome {
    if confirmations >= required {
        // Fully confirmed — finalize
        if let Err(e) = svc
            .repo()
            .update_tx_confirmed(
                tx.id,
                block as i64,
                tx.gas_used.unwrap_or(0),
                confirmations as i32,
            )
            .await
        {
            tracing::warn!(tx_id = %tx.id, error = %e, "failed_to_confirm_tx");
            return TxOutcome::Pending;
        }

        tracing::info!(
            tx_id = %tx.id,
            chain = %tx.chain,
            tx_hash = %tx_hash,
            block,
            confirmations,
            required,
            "tx_confirmed"
        );

        // Post-confirmation: update the linked fill's on-chain tx hash
        post_confirmation(svc, tx, tx_hash).await;

        TxOutcome::Confirmed
    } else {
        // Partially confirmed — update count
        let _ = svc
            .repo()
            .increment_confirmations(tx.id, confirmations as i32)
            .await;

        tracing::debug!(
            tx_id = %tx.id,
            chain = %tx.chain,
            confirmations,
            required,
            "tx_confirming"
        );

        TxOutcome::Pending
    }
}

async fn handle_failed(
    svc: &WalletService,
    tx: &TransactionRecord,
    tx_hash: &str,
    reason: &str,
) -> TxOutcome {
    let _ = svc.repo().update_tx_failed(tx.id, reason).await;

    tracing::warn!(
        tx_id = %tx.id,
        chain = %tx.chain,
        tx_hash = %tx_hash,
        reason = %reason,
        "tx_failed_on_chain"
    );

    TxOutcome::Failed
}

async fn handle_pending(
    svc: &WalletService,
    tx: &TransactionRecord,
    tx_hash: &str,
    drop_timeout: i64,
) -> TxOutcome {
    let Some(submitted) = tx.submitted_at else {
        return TxOutcome::Pending;
    };

    let age_secs = (chrono::Utc::now() - submitted).num_seconds();
    if age_secs <= drop_timeout {
        return TxOutcome::Pending;
    }

    // Transaction exceeded the chain's drop timeout
    let reason = format!(
        "No receipt after {}s (chain drop timeout: {}s)",
        age_secs, drop_timeout
    );

    let _ = svc.repo().update_tx_dropped(tx.id, &reason).await;

    tracing::warn!(
        tx_id = %tx.id,
        chain = %tx.chain,
        tx_hash = %tx_hash,
        age_secs,
        drop_timeout,
        "tx_dropped"
    );

    TxOutcome::Dropped
}

// ── Post-confirmation hook ───────────────────────────────

/// After a transaction is confirmed on-chain, update the linked fill
/// and execution records to reflect the on-chain settlement.
async fn post_confirmation(svc: &WalletService, tx: &TransactionRecord, tx_hash: &str) {
    let Some(fill_id) = tx.fill_id else {
        return;
    };

    // Update the fill's tx_hash to the on-chain hash
    if let Err(e) = svc.repo().mark_fill_on_chain_settled(fill_id, tx_hash).await {
        tracing::warn!(
            fill_id = %fill_id,
            error = %e,
            "failed_to_update_fill_tx_hash"
        );
    }

    // Update execution status if there's a matching execution record
    update_execution_status(svc, fill_id, tx_hash).await;
}

async fn update_execution_status(svc: &WalletService, fill_id: Uuid, tx_hash: &str) {
    // Best-effort: update the execution record linked to this fill
    let result = sqlx::query(
        "UPDATE executions SET status = 'completed', tx_hash = $2
         WHERE intent_id = (SELECT intent_id FROM fills WHERE id = $1)
           AND status != 'completed'",
    )
    .bind(fill_id)
    .bind(tx_hash)
    .execute(svc.repo().pool())
    .await;

    if let Err(e) = result {
        tracing::warn!(
            fill_id = %fill_id,
            error = %e,
            "failed_to_update_execution_status"
        );
    }
}
