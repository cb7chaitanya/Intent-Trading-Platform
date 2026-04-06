//! Background worker that orchestrates cross-chain settlements via bridge adapters.
//!
//! Five-phase poll cycle:
//! 1. Lock: pending source legs → bridge.lock_funds → Escrowed
//! 2. Verify: escrowed source legs → bridge.verify_lock → Confirmed
//! 3. Release: ready destination legs → bridge.release_funds → Executing
//! 4. Timeout: expired legs → refund + cascade
//! 5. Finalize: both legs confirmed → mark intent Completed

use std::sync::Arc;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::metrics::{counters, gauges};
use crate::models::intent::IntentStatus;

use super::bridge::{BridgeStatus, BridgeTransferParams};
use super::bridge_registry::BridgeRegistry;
use super::model::LegStatus;
use super::service::CrossChainService;

const POLL_INTERVAL_SECS: u64 = 5;

// ── Entry point ──────────────────────────────────────────

pub async fn run(
    service: Arc<CrossChainService>,
    bridges: Arc<BridgeRegistry>,
    pool: PgPool,
    cancel: CancellationToken,
) {
    tracing::info!(
        poll_secs = POLL_INTERVAL_SECS,
        bridges = ?bridges.list_bridges(),
        "Cross-chain settlement worker started"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)) => {
                let start = std::time::Instant::now();

                let locked = lock_pending_sources(&service, &bridges).await;
                let verified = verify_escrowed(&service, &bridges).await;
                let released = release_destinations(&service, &bridges).await;
                let timeouts = process_timeouts(&service).await;
                let completed = finalize_completed(&service, &pool).await;
                update_gauge(&service).await;

                let ms = start.elapsed().as_millis();
                if locked + verified + released + timeouts + completed > 0 {
                    tracing::info!(locked, verified, released, timeouts, completed, ms, "cross_chain_cycle");
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("Cross-chain settlement worker shutting down");
                return;
            }
        }
    }
}

// ── Phase 1: Lock source legs via bridge ─────────────────

#[tracing::instrument(skip_all, name = "cross_chain.lock_sources")]
async fn lock_pending_sources(svc: &CrossChainService, bridges: &BridgeRegistry) -> u32 {
    let legs = match svc.find_pending_source_legs().await {
        Ok(l) => l,
        Err(e) => { tracing::error!(error = %e, "pending_source_query_failed"); return 0; }
    };

    let mut n = 0u32;
    for leg in &legs {
        let settlement = match svc.get_settlement(leg.fill_id).await {
            Ok(Some(s)) => s, _ => continue,
        };
        let dest = &settlement.destination_leg;

        let bridge = match bridges.find(&leg.chain, &dest.chain) {
            Ok(b) => b,
            Err(e) => { tracing::warn!(leg_id = %leg.id, error = %e, "no_bridge"); continue; }
        };

        let params = BridgeTransferParams {
            source_chain: leg.chain.clone(),
            dest_chain: dest.chain.clone(),
            token: leg.token_mint.clone().unwrap_or_default(),
            amount: leg.amount as u64,
            sender: leg.from_address.clone(),
            recipient: dest.to_address.clone(),
        };

        match bridge.lock_funds(&params).await {
            Ok(receipt) => {
                if svc.execute_source_leg(leg.id, &receipt.tx_hash).await.is_ok() {
                    tracing::info!(leg_id = %leg.id, bridge = bridge.name(), tx = %receipt.tx_hash, "bridge_locked");
                    counters::CROSS_CHAIN_LEGS_PROCESSED.with_label_values(&["locked"]).inc();
                    n += 1;
                }
            }
            Err(e) => {
                tracing::error!(leg_id = %leg.id, bridge = bridge.name(), error = %e, "bridge_lock_failed");
                let _ = svc.fail_leg(leg.id, &e.to_string()).await;
            }
        }
    }
    n
}

// ── Phase 2: Verify escrowed source legs ─────────────────

#[tracing::instrument(skip_all, name = "cross_chain.verify_escrowed")]
async fn verify_escrowed(svc: &CrossChainService, bridges: &BridgeRegistry) -> u32 {
    let legs = match svc.find_escrowed_source_legs().await {
        Ok(l) => l,
        Err(e) => { tracing::error!(error = %e, "escrowed_query_failed"); return 0; }
    };

    let mut n = 0u32;
    for leg in &legs {
        let tx_hash = match &leg.tx_hash { Some(h) => h.as_str(), None => continue };
        let settlement = match svc.get_settlement(leg.fill_id).await { Ok(Some(s)) => s, _ => continue };
        let bridge = match bridges.find(&leg.chain, &settlement.destination_leg.chain) { Ok(b) => b, Err(_) => continue };

        match bridge.verify_lock(tx_hash).await {
            Ok(BridgeStatus::Completed { .. } | BridgeStatus::InTransit { .. }) => {
                if svc.confirm_leg(leg.id).await.is_ok() {
                    tracing::info!(leg_id = %leg.id, bridge = bridge.name(), "source_confirmed");
                    n += 1;
                }
            }
            Ok(BridgeStatus::Failed { reason }) => {
                let _ = svc.fail_leg(leg.id, &reason).await;
                tracing::warn!(leg_id = %leg.id, reason, "bridge_lock_failed_on_chain");
            }
            _ => {}
        }
    }
    n
}

// ── Phase 3: Release on destination chain ────────────────

#[tracing::instrument(skip_all, name = "cross_chain.release_destinations")]
async fn release_destinations(svc: &CrossChainService, bridges: &BridgeRegistry) -> u32 {
    let legs = match svc.find_ready_destination_legs().await {
        Ok(l) => l,
        Err(e) => { tracing::error!(error = %e, "ready_dest_query_failed"); return 0; }
    };

    let mut n = 0u32;
    for leg in &legs {
        let settlement = match svc.get_settlement(leg.fill_id).await { Ok(Some(s)) => s, _ => continue };
        let source = &settlement.source_leg;
        let bridge = match bridges.find(&source.chain, &leg.chain) { Ok(b) => b, Err(_) => continue };

        let params = BridgeTransferParams {
            source_chain: source.chain.clone(),
            dest_chain: leg.chain.clone(),
            token: leg.token_mint.clone().unwrap_or_default(),
            amount: leg.amount as u64,
            sender: source.from_address.clone(),
            recipient: leg.to_address.clone(),
        };
        let msg_id = format!("msg_{}", source.tx_hash.as_deref().unwrap_or(""));

        match bridge.release_funds(&params, &msg_id).await {
            Ok(dest_tx) => {
                if svc.mark_executing(leg.id, &dest_tx).await.is_ok() {
                    tracing::info!(leg_id = %leg.id, bridge = bridge.name(), dest_tx = %dest_tx, "bridge_released");
                    counters::CROSS_CHAIN_LEGS_PROCESSED.with_label_values(&["executing"]).inc();
                    n += 1;
                }
            }
            Err(e) => {
                tracing::error!(leg_id = %leg.id, error = %e, "bridge_release_failed");
                let _ = svc.fail_leg(leg.id, &e.to_string()).await;
            }
        }
    }
    n
}

// ── Phase 4: Timeouts ────────────────────────────────────

async fn process_timeouts(svc: &CrossChainService) -> u32 {
    let legs = match svc.find_timed_out_legs().await {
        Ok(l) => l,
        Err(e) => { tracing::error!(error = %e, "timeout_query_failed"); return 0; }
    };

    let mut n = 0u32;
    for leg in &legs {
        tracing::warn!(leg_id = %leg.id, chain = %leg.chain, leg_index = leg.leg_index, "timeout");
        if svc.refund_leg(leg.id).await.is_err() { continue; }

        counters::CROSS_CHAIN_TIMEOUTS_TOTAL.inc();
        counters::CROSS_CHAIN_LEGS_PROCESSED.with_label_values(&["refunded"]).inc();
        n += 1;

        if leg.leg_index == 0 && leg.status == LegStatus::Escrowed {
            if let Ok(Some(s)) = svc.get_settlement(leg.fill_id).await {
                if matches!(s.destination_leg.status, LegStatus::Pending | LegStatus::Executing) {
                    let _ = svc.refund_leg(s.destination_leg.id).await;
                    counters::CROSS_CHAIN_LEGS_PROCESSED.with_label_values(&["refunded"]).inc();
                    n += 1;
                }
            }
        }
    }
    n
}

// ── Phase 5: Finalize ────────────────────────────────────

async fn finalize_completed(svc: &CrossChainService, pool: &PgPool) -> u32 {
    let rows = match sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        "SELECT DISTINCT l.fill_id, l.intent_id
         FROM cross_chain_legs l
         WHERE l.leg_index = 0 AND l.status = 'confirmed'
           AND EXISTS (SELECT 1 FROM cross_chain_legs l2 WHERE l2.fill_id = l.fill_id AND l2.leg_index = 1 AND l2.status = 'confirmed')
           AND EXISTS (SELECT 1 FROM intents i WHERE i.id = l.intent_id AND i.status != 'completed')
         LIMIT 50",
    )
    .fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => { tracing::error!(error = %e, "finalize_query_failed"); return 0; }
    };

    let mut n = 0u32;
    for (fill_id, intent_id) in &rows {
        if let Ok(r) = sqlx::query("UPDATE intents SET status = $1 WHERE id = $2 AND status != 'completed'")
            .bind(IntentStatus::Completed).bind(intent_id).execute(pool).await
        {
            if r.rows_affected() > 0 {
                tracing::info!(intent_id = %intent_id, fill_id = %fill_id, "cross_chain_completed");
                counters::CROSS_CHAIN_LEGS_PROCESSED.with_label_values(&["confirmed"]).inc();
                n += 1;
            }
        }
    }
    n
}

// ── Gauge ────────────────────────────────────────────────

async fn update_gauge(svc: &CrossChainService) {
    let t = svc.find_timed_out_legs().await.map(|v| v.len()).unwrap_or(0);
    let r = svc.find_ready_destination_legs().await.map(|v| v.len()).unwrap_or(0);
    gauges::CROSS_CHAIN_PENDING_LEGS.set((t + r) as i64);
}
