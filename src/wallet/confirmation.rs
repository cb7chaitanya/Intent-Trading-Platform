use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::model::TxStatus;
use super::service::WalletService;

/// Required number of block confirmations before finalizing.
const REQUIRED_CONFIRMATIONS: i32 = 12;

/// Poll interval for checking transaction status.
const POLL_INTERVAL_SECS: u64 = 5;

/// Background worker that polls submitted transactions for confirmations.
///
/// Flow:
/// 1. Fetch all transactions with status `submitted`
/// 2. For each, call `eth_getTransactionReceipt` via RPC
/// 3. If receipt exists: update block_number, gas_used, confirmations
/// 4. If confirmations >= REQUIRED_CONFIRMATIONS: mark `confirmed`
/// 5. If receipt shows failure: mark `failed`
/// 6. Sleep and repeat
pub async fn run(
    wallet_service: Arc<WalletService>,
    cancel: CancellationToken,
) {
    tracing::info!(
        required_confirmations = REQUIRED_CONFIRMATIONS,
        poll_secs = POLL_INTERVAL_SECS,
        "Confirmation worker started"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                if let Err(e) = poll_pending(&wallet_service).await {
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

async fn poll_pending(svc: &WalletService) -> Result<(), String> {
    let pending = svc
        .get_pending_transactions()
        .await
        .map_err(|e| e.to_string())?;

    if pending.is_empty() {
        return Ok(());
    }

    let current_block = svc
        .rpc()
        .get_block_number()
        .await
        .map_err(|e| e.to_string())?;

    for tx in &pending {
        // Only check transactions that have been submitted (have a tx_hash)
        if tx.status != TxStatus::Submitted {
            continue;
        }
        let tx_hash = match &tx.tx_hash {
            Some(h) => h,
            None => continue,
        };

        match svc.rpc().get_transaction_receipt(tx_hash).await {
            Ok(Some(receipt)) => {
                if !receipt.status {
                    // Transaction reverted on-chain
                    if let Err(e) = svc
                        .repo()
                        .update_tx_failed(tx.id, "Transaction reverted on-chain")
                        .await
                    {
                        tracing::warn!(tx_id = %tx.id, error = %e, "failed_to_mark_tx_failed");
                    }
                    tracing::warn!(
                        tx_id = %tx.id,
                        tx_hash = %tx_hash,
                        "tx_reverted_on_chain"
                    );
                    continue;
                }

                let confirmations =
                    (current_block - receipt.block_number).max(0) as i32;

                if confirmations >= REQUIRED_CONFIRMATIONS {
                    // Fully confirmed
                    if let Err(e) = svc
                        .repo()
                        .update_tx_confirmed(
                            tx.id,
                            receipt.block_number,
                            receipt.gas_used,
                            confirmations,
                        )
                        .await
                    {
                        tracing::warn!(tx_id = %tx.id, error = %e, "failed_to_confirm_tx");
                    } else {
                        tracing::info!(
                            tx_id = %tx.id,
                            tx_hash = %tx_hash,
                            block = receipt.block_number,
                            confirmations,
                            "tx_confirmed"
                        );
                    }
                } else {
                    // Partially confirmed — update count
                    let _ = svc
                        .repo()
                        .increment_confirmations(tx.id, confirmations)
                        .await;
                }
            }
            Ok(None) => {
                // Not yet mined — check if too old (dropped)
                if let Some(submitted) = tx.submitted_at {
                    let age_secs = (chrono::Utc::now() - submitted).num_seconds();
                    if age_secs > 600 {
                        // 10 minutes without receipt → likely dropped
                        let _ = svc
                            .repo()
                            .update_tx_failed(tx.id, "Transaction dropped (no receipt after 10m)")
                            .await;
                        tracing::warn!(
                            tx_id = %tx.id,
                            tx_hash = %tx_hash,
                            age_secs,
                            "tx_likely_dropped"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    tx_id = %tx.id,
                    tx_hash = %tx_hash,
                    error = %e,
                    "receipt_fetch_failed"
                );
            }
        }
    }

    Ok(())
}
