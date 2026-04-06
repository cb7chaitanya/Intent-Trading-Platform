use sqlx::PgPool;
use uuid::Uuid;

use super::model::Market;

pub struct MarketRepository {
    pool: PgPool,
}

impl MarketRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, market: &Market) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO markets
                (id, base_asset, quote_asset, tick_size, min_order_size, fee_rate,
                 chain, settlement_contract, base_token_mint, quote_token_mint,
                 base_decimals, quote_decimals, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        )
        .bind(market.id)
        .bind(&market.base_asset)
        .bind(&market.quote_asset)
        .bind(market.tick_size)
        .bind(market.min_order_size)
        .bind(market.fee_rate)
        .bind(&market.chain)
        .bind(&market.settlement_contract)
        .bind(&market.base_token_mint)
        .bind(&market.quote_token_mint)
        .bind(market.base_decimals)
        .bind(market.quote_decimals)
        .bind(market.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Market>, sqlx::Error> {
        sqlx::query_as::<_, Market>("SELECT * FROM markets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn find_all(&self) -> Result<Vec<Market>, sqlx::Error> {
        sqlx::query_as::<_, Market>("SELECT * FROM markets ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn find_by_chain(&self, chain: &str) -> Result<Vec<Market>, sqlx::Error> {
        sqlx::query_as::<_, Market>("SELECT * FROM markets WHERE chain = $1 ORDER BY created_at DESC")
            .bind(chain)
            .fetch_all(&self.pool)
            .await
    }
}
