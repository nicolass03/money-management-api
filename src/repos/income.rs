use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, IncomeRow, IncomeSource};
use crate::repos::{connection, settings};
use crate::schema::income;
use crate::state::DbPool;

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<IncomeRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_all_with_conn(&mut conn, user_id).await
}

pub async fn list_all_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<IncomeRow>, ApiError> {
    income::table
        .filter(income::user_id.eq(user_id))
        .order(income::date.desc())
        .select(IncomeRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<IncomeRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
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
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::insert_into(income::table)
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
                .get_result(conn)
                .await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<IncomeRow, diesel::result::Error>(row)
        })
    })
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
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::update(
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
            .get_result(conn)
            .await
            .optional()?;
            if row.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<IncomeRow>, diesel::result::Error>(row)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let deleted = diesel::delete(
                income::table
                    .filter(income::user_id.eq(user_id))
                    .filter(income::id.eq(id)),
            )
            .execute(conn)
            .await?;
            if deleted > 0 {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}
