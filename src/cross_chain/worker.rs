//! Background worker that monitors cross-chain settlement legs.
//!
//! Runs on a tokio interval loop with graceful shutdown support.
//!
//! Each cycle:
//! 1. Scan for timed-out legs → refund both legs of the settlement.
//! 2. Scan for destination legs ready to execute (source escrowed/confirmed)
//!    → mark as Executing for wallet service pickup.
//! 3. Scan for fully-confirmed settlements → mark intent Completed.
//! 4. Update Prometheus gauges and counters.

use std::sync::Arc;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::metrics::{counters, gauges};
use crate::models::intent::IntentStatus;

use super::model::LegStatus;
use super::service::CrossChainService;

const POLL_INTERVAL_SECS: u64 = 5;

// ── Entry point ──────────────────────────────────────────

pub async fn run(
    service: Arc<CrossChainService>,
    pool: PgPool,
    cancel: CancellationToken,
) {
    tracing::info!(
        poll_secs = POLL_INTERVAL_SECS,
        "Cross-chain settlement worker started"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                let cycle_start = std::time::Instant::now();

                let timeouts = process_timeouts(&service).await;
                let destinations = process_ready_destinations(&service).await;
                let completed = finalize_completed(&service, &pool).await;
                update_pending_gauge(&service).await;

                let cycle_ms = cycle_start.elapsed().as_millis();
                if timeouts + destinations + completed > 0 {
                    tracing::info!(
                        timeouts,
                        destinations,
                        completed,
                        cycle_ms,
                        "cross_chain_worker_cycle"
                    );
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Cross-chain settlement worker shutting down");
                return;
            }
        }
    }
}

// ── Timeout processing ───────────────────────────────────

async fn process_timeouts(svc: &CrossChainService) -> u32 {
    let timed_out = match svc.find_timed_out_legs().await {
        Ok(legs) => legs,
        Err(e) => {
            tracing::error!(error = %e, "cross_chain_timeout_query_failed");
            return 0;
        }
    };

    let mut count = 0u32;

    for leg in &timed_out {
        tracing::warn!(
            leg_id = %leg.id,
            intent_id = %leg.intent_id,
            fill_id = %leg.fill_id,
            chain = %leg.chain,
            leg_index = leg.leg_index,
            status = ?leg.status,
            "cross_chain_leg_timeout"
        );

        // Refund this leg
        if let Err(e) = svc.refund_leg(leg.id).await {
            tracing::error!(leg_id = %leg.id, error = %e, "cross_chain_refund_failed");
            continue;
        }

        counters::CROSS_CHAIN_TIMEOUTS_TOTAL.inc();
        counters::CROSS_CHAIN_LEGS_PROCESSED
            .with_label_values(&["refunded"])
            .inc();
        count += 1;

        // If source was escrowed, also refund the pending destination
        if leg.leg_index == 0 && leg.status == LegStatus::Escrowed {
            if let Ok(Some(settlement)) = svc.get_settlement(leg.fill_id).await {
                let dest = &settlement.destination_leg;
                if dest.status == LegStatus::Pending || dest.status == LegStatus::Executing {
                    if let Err(e) = svc.refund_leg(dest.id).await {
                        tracing::error!(
                            leg_id = %dest.id,
                            error = %e,
                            "cross_chain_counterpart_refund_failed"
                        );
                    } else {
                        counters::CROSS_CHAIN_LEGS_PROCESSED
                            .with_label_values(&["refunded"])
                            .inc();
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

// ── Destination leg execution ────────────────────────────

async fn process_ready_destinations(svc: &CrossChainService) -> u32 {
    let ready = match svc.find_ready_destination_legs().await {
        Ok(legs) => legs,
        Err(e) => {
            tracing::error!(error = %e, "cross_chain_ready_query_failed");
            return 0;
        }
    };

    let mut count = 0u32;

    for leg in &ready {
        tracing::info!(
            leg_id = %leg.id,
            intent_id = %leg.intent_id,
            fill_id = %leg.fill_id,
            chain = %leg.chain,
            amount = leg.amount,
            "cross_chain_destination_ready"
        );

        if let Err(e) = svc.mark_executing(leg.id, "worker_dispatched").await {
            tracing::error!(
                leg_id = %leg.id,
                error = %e,
                "cross_chain_mark_executing_failed"
            );
            continue;
        }

        counters::CROSS_CHAIN_LEGS_PROCESSED
            .with_label_values(&["executing"])
            .inc();
        count += 1;
    }

    count
}

// ── Finalize completed settlements ───────────────────────

/// Find settlements where both legs are confirmed and mark the intent Completed.
async fn finalize_completed(svc: &CrossChainService, pool: &PgPool) -> u32 {
    // Find fills that have two confirmed legs but the intent is still not Completed
    let rows = match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        "SELECT DISTINCT l.fill_id, l.intent_id
         FROM cross_chain_legs l
         WHERE l.leg_index = 0
           AND l.status = 'confirmed'
           AND EXISTS (
               SELECT 1 FROM cross_chain_legs l2
               WHERE l2.fill_id = l.fill_id AND l2.leg_index = 1 AND l2.status = 'confirmed'
           )
           AND EXISTS (
               SELECT 1 FROM intents i
               WHERE i.id = l.intent_id AND i.status != 'completed'
           )
         LIMIT 50",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "cross_chain_finalize_query_failed");
            return 0;
        }
    };

    let mut count = 0u32;

    for (fill_id, intent_id) in &rows {
        // Mark intent Completed
        let result = sqlx::query(
            "UPDATE intents SET status = $1 WHERE id = $2 AND status != 'completed'",
        )
        .bind(IntentStatus::Completed)
        .bind(intent_id)
        .execute(pool)
        .await;

        match result {
            Ok(r) if r.rows_affected() > 0 => {
                tracing::info!(
                    intent_id = %intent_id,
                    fill_id = %fill_id,
                    "cross_chain_settlement_completed"
                );
                counters::CROSS_CHAIN_LEGS_PROCESSED
                    .with_label_values(&["confirmed"])
                    .inc();
                count += 1;
            }
            Ok(_) => {} // already completed
            Err(e) => {
                tracing::error!(
                    intent_id = %intent_id,
                    error = %e,
                    "cross_chain_intent_complete_failed"
                );
            }
        }
    }

    count
}

// ── Gauge update ─────────────────────────────────────────

async fn update_pending_gauge(svc: &CrossChainService) {
    // Count legs in non-terminal states
    let timed_out = svc.find_timed_out_legs().await.map(|v| v.len()).unwrap_or(0);
    let ready = svc.find_ready_destination_legs().await.map(|v| v.len()).unwrap_or(0);
    gauges::CROSS_CHAIN_PENDING_LEGS.set((timed_out + ready) as i64);
}
