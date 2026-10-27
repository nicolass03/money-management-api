use chrono::{NaiveDate, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PayFrequency, RecurringTransferRow};
use crate::repos::{connection, settings};
use crate::schema::recurring_transfers;
use crate::state::DbPool;

/// The writable fields of a recurring transfer template (insert values and edit changeset).
#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = recurring_transfers, treat_none_as_null = true)]
pub struct RecurringTransferFields {
    pub name: Option<String>,
    pub from_account_id: Uuid,
    pub to_account_id: Uuid,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
}

/// Active (not soft-deleted) templates. Used by the list endpoint and the daily job.
pub async fn list_all_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<RecurringTransferRow>, ApiError> {
    recurring_transfers::table
        .filter(recurring_transfers::user_id.eq(user_id))
        .filter(recurring_transfers::deleted_at.is_null())
        .order(recurring_transfers::created_at.asc())
        .select(RecurringTransferRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<RecurringTransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_all_with_conn(&mut conn, user_id).await
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<RecurringTransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    recurring_transfers::table
        .filter(recurring_transfers::user_id.eq(user_id))
        .filter(recurring_transfers::id.eq(id))
        .filter(recurring_transfers::deleted_at.is_null())
        .select(RecurringTransferRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    fields: &RecurringTransferFields,
) -> Result<RecurringTransferRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::insert_into(recurring_transfers::table)
                .values((fields, recurring_transfers::user_id.eq(user_id)))
                .returning(RecurringTransferRow::as_returning())
                .get_result(conn)
                .await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<RecurringTransferRow, diesel::result::Error>(row)
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Edits a template. Bumping `updated_at` stops the daily job from materializing any occurrence
/// dated before today's edit, so an edit never backfills history (as with recurring expenses).
pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    fields: &RecurringTransferFields,
) -> Result<Option<RecurringTransferRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let row = diesel::update(
                recurring_transfers::table
                    .filter(recurring_transfers::user_id.eq(user_id))
                    .filter(recurring_transfers::id.eq(id))
                    .filter(recurring_transfers::deleted_at.is_null()),
            )
            .set((fields, recurring_transfers::updated_at.eq(now)))
            .returning(RecurringTransferRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if row.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<RecurringTransferRow>, diesel::result::Error>(row)
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Soft-deletes a template: future occurrences stop, materialized transfers are kept. Returns
/// whether a row was deleted.
pub async fn soft_delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<bool, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let updated = diesel::update(
                recurring_transfers::table
                    .filter(recurring_transfers::user_id.eq(user_id))
                    .filter(recurring_transfers::id.eq(id))
                    .filter(recurring_transfers::deleted_at.is_null()),
            )
            .set((
                recurring_transfers::deleted_at.eq(now),
                recurring_transfers::updated_at.eq(now),
            ))
            .execute(conn)
            .await?;
            if updated > 0 {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<bool, diesel::result::Error>(updated > 0)
        })
    })
    .await
    .map_err(ApiError::from)
}
