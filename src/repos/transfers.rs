use chrono::{NaiveDate, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, TransferRow};
use crate::repos::{connection, settings};
use crate::schema::transfers;
use crate::state::DbPool;

/// The writable fields of a transfer: the insert values and the full-edit changeset.
/// `treat_none_as_null` so clearing the name on edit really clears it.
#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = transfers, treat_none_as_null = true)]
pub struct TransferFields {
    pub name: Option<String>,
    pub from_account_id: Uuid,
    pub to_account_id: Uuid,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub date: NaiveDate,
}

/// Active (not soft-deleted) transfers, newest first.
pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<TransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    transfers::table
        .filter(transfers::user_id.eq(user_id))
        .filter(transfers::deleted_at.is_null())
        .order((transfers::date.desc(), transfers::created_at.desc()))
        .select(TransferRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}

/// Every transfer that moves money between `excluded` accounts and the rest — soft-deleted
/// tombstones included, flagged `true` — for projections: the live rows count as money in or out
/// of the projected accounts, and every row's `(recurring_transfer_id, date)` slot keeps that
/// occurrence from being projected again.
pub async fn list_crossing_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    excluded: &[Uuid],
) -> Result<Vec<(TransferRow, bool)>, ApiError> {
    let from_excluded = transfers::from_account_id.eq_any(excluded);
    let to_excluded = transfers::to_account_id.eq_any(excluded);
    transfers::table
        .filter(transfers::user_id.eq(user_id))
        .filter(from_excluded.ne(to_excluded))
        .select((TransferRow::as_select(), transfers::deleted_at.is_not_null()))
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<TransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    transfers::table
        .filter(transfers::user_id.eq(user_id))
        .filter(transfers::id.eq(id))
        .filter(transfers::deleted_at.is_null())
        .select(TransferRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    fields: &TransferFields,
) -> Result<TransferRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::insert_into(transfers::table)
                .values((fields, transfers::user_id.eq(user_id)))
                .returning(TransferRow::as_returning())
                .get_result(conn)
                .await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<TransferRow, diesel::result::Error>(row)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    fields: &TransferFields,
) -> Result<Option<TransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::update(
                transfers::table
                    .filter(transfers::user_id.eq(user_id))
                    .filter(transfers::id.eq(id))
                    .filter(transfers::deleted_at.is_null()),
            )
            .set(fields)
            .returning(TransferRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if row.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<TransferRow>, diesel::result::Error>(row)
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Soft-deletes a transfer (the only delete path). For a recurring occurrence the tombstone keeps
/// its `(recurring_transfer_id, date)` slot, so the daily job never re-creates it. Returns the
/// deleted transfer's date, or `None` when no row was deleted.
pub async fn soft_delete(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<NaiveDate>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let date = diesel::update(
                transfers::table
                    .filter(transfers::user_id.eq(user_id))
                    .filter(transfers::id.eq(id))
                    .filter(transfers::deleted_at.is_null()),
            )
            .set(transfers::deleted_at.eq(now))
            .returning(transfers::date)
            .get_result(conn)
            .await
            .optional()?;
            if date.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<NaiveDate>, diesel::result::Error>(date)
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Inserts one materialized occurrence of a recurring transfer (daily job). `ON CONFLICT DO
/// NOTHING` against the `(recurring_transfer_id, date)` unique index — tombstones included — makes
/// a repeat run a no-op. Returns whether a row was inserted.
pub async fn insert_scheduled(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    recurring_transfer_id: Uuid,
    fields: &TransferFields,
) -> Result<bool, diesel::result::Error> {
    let inserted = diesel::insert_into(transfers::table)
        .values((
            fields,
            transfers::user_id.eq(user_id),
            transfers::recurring_transfer_id.eq(recurring_transfer_id),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(inserted > 0)
}
