//! Background worker that monitors cross-chain settlement legs.
//!
//! Responsibilities:
//! 1. Trigger destination leg execution after source leg is confirmed.
//! 2. Detect timeouts and initiate refunds.
//! 3. Update intent status when both legs are confirmed.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::model::LegStatus;
use super::service::CrossChainService;

const POLL_INTERVAL_SECS: u64 = 5;

pub async fn run(service: Arc<CrossChainService>, cancel: CancellationToken) {
    tracing::info!("Cross-chain settlement worker started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                process_timeouts(&service).await;
                process_ready_destinations(&service).await;
            }
            _ = cancel.cancelled() => {
                tracing::info!("Cross-chain settlement worker shutting down");
                return;
            }
        }
    }
}

/// Find timed-out legs and refund them.
async fn process_timeouts(svc: &CrossChainService) {
    let timed_out = match svc.find_timed_out_legs().await {
        Ok(legs) => legs,
        Err(e) => {
            tracing::error!(error = %e, "Failed to query timed-out legs");
            return;
        }
    };

    for leg in &timed_out {
        tracing::warn!(
            leg_id = %leg.id,
            intent_id = %leg.intent_id,
            chain = %leg.chain,
            leg_index = leg.leg_index,
            status = ?leg.status,
            "cross_chain_leg_timeout"
        );

        // Refund: mark the leg as refunded
        if let Err(e) = svc.refund_leg(leg.id).await {
            tracing::error!(
                leg_id = %leg.id,
                error = %e,
                "cross_chain_refund_failed"
            );
        }

        // If the source leg was escrowed but timed out, also refund the counterpart
        if leg.leg_index == 0 && leg.status == LegStatus::Escrowed {
            if let Ok(Some(settlement)) = svc.get_settlement(leg.fill_id).await {
                if settlement.destination_leg.status == LegStatus::Pending {
                    let _ = svc.refund_leg(settlement.destination_leg.id).await;
                }
            }
        }
    }

    if !timed_out.is_empty() {
        tracing::info!(
            count = timed_out.len(),
            "cross_chain_timeouts_processed"
        );
    }
}

/// Find destination legs ready for execution (source confirmed) and mark them.
async fn process_ready_destinations(svc: &CrossChainService) {
    let ready = match svc.find_ready_destination_legs().await {
        Ok(legs) => legs,
        Err(e) => {
            tracing::error!(error = %e, "Failed to query ready destination legs");
            return;
        }
    };

    for leg in &ready {
        tracing::info!(
            leg_id = %leg.id,
            intent_id = %leg.intent_id,
            chain = %leg.chain,
            "cross_chain_destination_ready"
        );

        // The actual execution is handled by the settlement engine / wallet service.
        // Here we just mark the leg as executing to signal it's picked up.
        // The wallet service will call mark_executing → confirm_leg when the tx lands.
        if let Err(e) = svc.mark_executing(leg.id, "awaiting_tx").await {
            tracing::error!(
                leg_id = %leg.id,
                error = %e,
                "failed_to_mark_destination_executing"
            );
        }
    }
}
