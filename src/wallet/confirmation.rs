use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::chain::TxState;
use super::model::TxStatus;
use super::service::WalletService;

/// Poll interval for checking transaction status.
const POLL_INTERVAL_SECS: u64 = 5;

/// Background worker that polls submitted transactions for confirmations.
/// Supports multiple chains by routing each transaction through its
/// chain's adapter from the ChainRegistry.
pub async fn run(wallet_service: Arc<WalletService>, cancel: CancellationToken) {
    tracing::info!(
        poll_secs = POLL_INTERVAL_SECS,
        "Multi-chain confirmation worker started"
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

    for tx in &pending {
        if tx.status != TxStatus::Submitted {
            continue;
        }
        let tx_hash = match &tx.tx_hash {
            Some(h) => h,
            None => continue,
        };

        // Route to the correct chain adapter
        let adapter = match svc.chains().get(&tx.chain) {
            Ok(a) => a,
            Err(_) => {
                tracing::warn!(
                    tx_id = %tx.id,
                    chain = %tx.chain,
                    "no_adapter_for_chain_skipping"
                );
                continue;
            }
        };

        let required_confirmations = adapter.required_confirmations();

        match adapter.get_transaction_status(tx_hash).await {
            Ok(TxState::Confirmed {
                block,
                confirmations,
            }) => {
                if confirmations >= required_confirmations {
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
                    } else {
                        tracing::info!(
                            tx_id = %tx.id,
                            chain = %tx.chain,
                            tx_hash = %tx_hash,
                            block,
                            confirmations,
                            "tx_confirmed"
                        );
                    }
                } else {
                    let _ = svc
                        .repo()
                        .increment_confirmations(tx.id, confirmations as i32)
                        .await;
                }
            }
            Ok(TxState::Failed { reason }) => {
                let _ = svc.repo().update_tx_failed(tx.id, &reason).await;
                tracing::warn!(
                    tx_id = %tx.id,
                    chain = %tx.chain,
                    tx_hash = %tx_hash,
                    reason = %reason,
                    "tx_failed_on_chain"
                );
            }
            Ok(TxState::Pending) => {
                // Check for dropped transactions
                if let Some(submitted) = tx.submitted_at {
                    let age_secs = (chrono::Utc::now() - submitted).num_seconds();
                    if age_secs > 600 {
                        let _ = svc
                            .repo()
                            .update_tx_failed(
                                tx.id,
                                "Transaction dropped (no receipt after 10m)",
                            )
                            .await;
                        tracing::warn!(
                            tx_id = %tx.id,
                            chain = %tx.chain,
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
                    chain = %tx.chain,
                    tx_hash = %tx_hash,
                    error = %e,
                    "status_check_failed"
                );
            }
        }
    }

    Ok(())
}
