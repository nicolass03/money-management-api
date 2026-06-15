use std::collections::HashSet;

use chrono::{NaiveDate, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, IncomeRow, IncomeSource};
use crate::repos::{connection, settings};
use crate::schema::income;
use crate::state::DbPool;

/// Active (not soft-deleted) income rows — what clients see via `GET /income`.
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
        .filter(income::deleted_at.is_null())
        .order(income::date.desc())
        .select(IncomeRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

/// All income rows including soft-deleted tombstones. Projections need tombstones so a
/// deleted scheduled occurrence is neither counted nor re-projected from its schedule.
pub async fn list_with_deleted_with_conn(
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

/// Inserts a materialized scheduled income row for a due date. Used by the daily cron.
pub async fn insert_scheduled(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    date: NaiveDate,
    schedule_id: Uuid,
    created_at: chrono::DateTime<Utc>,
) -> Result<IncomeRow, diesel::result::Error> {
    diesel::insert_into(income::table)
        .values((
            income::user_id.eq(user_id),
            income::name.eq(name),
            income::amount.eq(amount),
            income::currency.eq(currency),
            income::source.eq(IncomeSource::Scheduled),
            income::date.eq(date),
            income::schedule_id.eq(schedule_id),
            income::created_at.eq(created_at),
        ))
        .returning(IncomeRow::as_returning())
        .get_result(conn)
        .await
}

/// Schedule ids that already have a materialized income row on `date` — including
/// soft-deleted tombstones — so the cron never re-creates a deleted occurrence.
pub async fn get_materialized_schedule_ids_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
) -> Result<HashSet<Uuid>, ApiError> {
    let due_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest("invalid date".into()))?;
    let mut conn = connection::user_connection(pool, user_id).await?;
    let rows: Vec<Option<Uuid>> = income::table
        .filter(income::user_id.eq(user_id))
        .filter(income::source.eq(IncomeSource::Scheduled))
        .filter(income::date.eq(due_date))
        .select(income::schedule_id)
        .load(&mut conn)
        .await?;
    Ok(rows.into_iter().flatten().collect())
}

/// Overrides the amount of a (materialized scheduled) income row, flagging the override
/// so future schedule edits leave the adjusted amount untouched (parity with expenses).
pub async fn update_amount(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    amount: i32,
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
                income::amount.eq(amount),
                income::amount_overridden.eq(true),
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

/// Soft-deletes a materialized scheduled income row. The tombstone keeps the
/// `(schedule_id, date)` slot occupied so the cron and projections skip it.
pub async fn soft_delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let updated = diesel::update(
                income::table
                    .filter(income::user_id.eq(user_id))
                    .filter(income::id.eq(id))
                    .filter(income::deleted_at.is_null()),
            )
            .set(income::deleted_at.eq(now))
            .execute(conn)
            .await?;
            if updated > 0 {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}
