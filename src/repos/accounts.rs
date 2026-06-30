use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{AccountRow, CurrencyCode};
use crate::repos::{connection, settings};
use crate::schema::accounts;
use crate::state::DbPool;

/// Active (non-archived) accounts, ordered by creation. Used by the API list endpoint, the
/// charge job's account picker, and the projection seed.
pub async fn list_active(pool: &DbPool, user_id: Uuid) -> Result<Vec<AccountRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_active_with_conn(&mut conn, user_id).await
}

pub async fn list_active_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<AccountRow>, ApiError> {
    accounts::table
        .filter(accounts::user_id.eq(user_id))
        .filter(accounts::archived_at.is_null())
        .order(accounts::created_at.asc())
        .select(AccountRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<AccountRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    accounts::table
        .filter(accounts::user_id.eq(user_id))
        .filter(accounts::id.eq(id))
        .select(AccountRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: Option<&str>,
    currency: CurrencyCode,
    initial_amount: i32,
) -> Result<AccountRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let account = diesel::insert_into(accounts::table)
                .values((
                    accounts::user_id.eq(user_id),
                    accounts::name.eq(name),
                    accounts::currency.eq(currency),
                    accounts::initial_amount.eq(initial_amount),
                    accounts::created_at.eq(now),
                ))
                .returning(AccountRow::as_returning())
                .get_result(conn)
                .await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<AccountRow, diesel::result::Error>(account)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    currency: CurrencyCode,
    initial_amount: i32,
) -> Result<Option<AccountRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let account = diesel::update(
                accounts::table
                    .filter(accounts::user_id.eq(user_id))
                    .filter(accounts::id.eq(id)),
            )
            .set((
                accounts::name.eq(name),
                accounts::currency.eq(currency),
                accounts::initial_amount.eq(initial_amount),
            ))
            .returning(AccountRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if account.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<AccountRow>, diesel::result::Error>(account)
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Soft-deletes (archives) an account. Assigned expenses/income keep pointing at it so history
/// and balances stay intact; the account simply drops out of pickers and the projection seed.
pub async fn archive(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<bool, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let updated = diesel::update(
                accounts::table
                    .filter(accounts::user_id.eq(user_id))
                    .filter(accounts::id.eq(id))
                    .filter(accounts::archived_at.is_null()),
            )
            .set(accounts::archived_at.eq(now))
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
