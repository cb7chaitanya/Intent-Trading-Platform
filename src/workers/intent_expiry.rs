use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::redis::{Event, EventBus};
use crate::db::stream_bus::{StreamBus, STREAM_INTENT_SETTLED};
use crate::models::intent::{Intent, IntentStatus};
use crate::settlement::worker::IntentSettledEvent;

const SCAN_INTERVAL_SECS: u64 = 30;
const BATCH_SIZE: i64 = 100;

pub async fn run(
    pool: PgPool,
    event_bus: Arc<Mutex<EventBus>>,
    stream_bus: Arc<StreamBus>,
    cancel: CancellationToken,
) {
    tracing::info!("Intent expiry worker started");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(SCAN_INTERVAL_SECS)) => {}
            _ = cancel.cancelled() => {
                tracing::info!("Intent expiry worker shutting down");
                return;
            }
        }

        if let Err(e) = expire_batch(&pool, &event_bus, &stream_bus).await {
            tracing::error!(error = %e, "Intent expiry cycle failed");
        }
    }
}

async fn expire_batch(
    pool: &PgPool,
    event_bus: &Arc<Mutex<EventBus>>,
    stream_bus: &Arc<StreamBus>,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().timestamp();

    let expired = sqlx::query_as::<_, Intent>(
        "SELECT * FROM intents
         WHERE deadline < $1
         AND status IN ('open', 'bidding')
         ORDER BY deadline ASC
         LIMIT $2",
    )
    .bind(now)
    .bind(BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    if expired.is_empty() {
        return Ok(());
    }

    tracing::info!(count = expired.len(), "Expiring intents past deadline");

    for intent in expired {
        if let Err(e) = expire_intent(pool, event_bus, stream_bus, &intent).await {
            tracing::error!(intent_id = %intent.id, error = %e, "Failed to expire intent");
        }
    }

    Ok(())
}

async fn expire_intent(
    pool: &PgPool,
    event_bus: &Arc<Mutex<EventBus>>,
    stream_bus: &Arc<StreamBus>,
    intent: &Intent,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tx = pool.begin().await?;

    let current = sqlx::query_as::<_, Intent>(
        "SELECT * FROM intents WHERE id = $1 FOR UPDATE",
    )
    .bind(intent.id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(current) = current else { return Ok(()); };

    if current.status != IntentStatus::Open && current.status != IntentStatus::Bidding {
        return Ok(());
    }

    sqlx::query("UPDATE intents SET status = $1 WHERE id = $2")
        .bind(IntentStatus::Expired)
        .bind(intent.id)
        .execute(&mut *tx)
        .await?;

    // Unlock balance
    let account_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT a.id FROM accounts a JOIN users u ON u.id = a.user_id
         WHERE u.id::text = $1 OR u.email = $1 LIMIT 1",
    )
    .bind(&intent.user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(account_id) = account_id {
        let asset_enum = match intent.token_in.to_uppercase().as_str() {
            "USDC" => "USDC", "ETH" => "ETH", "BTC" => "BTC", "SOL" => "SOL",
            _ => {
                tx.commit().await?;
                return Ok(());
            }
        };

        sqlx::query(
            "UPDATE balances SET available_balance = available_balance + $1,
                 locked_balance = locked_balance - $1, updated_at = NOW()
             WHERE account_id = $2 AND asset = $3::asset_type AND locked_balance >= $1",
        )
        .bind(intent.amount_in).bind(account_id).bind(asset_enum)
        .execute(&mut *tx).await?;
    }

    tx.commit().await?;

    // Publish Pub/Sub event
    let mut expired_intent = intent.clone();
    expired_intent.status = IntentStatus::Expired;
    let _ = event_bus.lock().await.publish(&Event::IntentExpired(expired_intent)).await;

    // Publish stream event for TWAP listener
    let settled_event = IntentSettledEvent {
        intent_id: intent.id,
        settled_qty: 0,
        status: "Expired".to_string(),
    };
    let _ = stream_bus.publish(STREAM_INTENT_SETTLED, &settled_event).await;

    tracing::info!(intent_id = %intent.id, "intent_expired");
    Ok(())
}
