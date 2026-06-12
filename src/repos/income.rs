use chrono::Utc;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, IncomeRow, IncomeSource};
use crate::schema::income;
use crate::state::DbPool;

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<IncomeRow>, ApiError> {
    let mut conn = pool.get().await?;
    income::table
        .filter(income::user_id.eq(user_id))
        .order(income::date.desc())
        .select(IncomeRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<IncomeRow>, ApiError> {
    let mut conn = pool.get().await?;
    income::table
        .filter(income::user_id.eq(user_id))
        .filter(income::id.eq(id))
        .select(IncomeRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    source: IncomeSource,
    date: chrono::NaiveDate,
) -> Result<IncomeRow, ApiError> {
    let mut conn = pool.get().await?;
    let now = Utc::now();
    diesel::insert_into(income::table)
        .values((
            income::user_id.eq(user_id),
            income::name.eq(name),
            income::amount.eq(amount),
            income::currency.eq(currency),
            income::source.eq(source),
            income::date.eq(date),
            income::schedule_id.eq::<Option<Uuid>>(None),
            income::created_at.eq(now),
        ))
        .returning(IncomeRow::as_returning())
        .get_result(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    source: IncomeSource,
    date: chrono::NaiveDate,
) -> Result<Option<IncomeRow>, ApiError> {
    let mut conn = pool.get().await?;
    diesel::update(
        income::table
            .filter(income::user_id.eq(user_id))
            .filter(income::id.eq(id)),
    )
    .set((
        income::name.eq(name),
        income::amount.eq(amount),
        income::currency.eq(currency),
        income::source.eq(source),
        income::date.eq(date),
    ))
    .returning(IncomeRow::as_returning())
    .get_result(&mut conn)
    .await
    .optional()
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = pool.get().await?;
    diesel::delete(
        income::table
            .filter(income::user_id.eq(user_id))
            .filter(income::id.eq(id)),
    )
    .execute(&mut conn)
    .await?;
    Ok(())
}
