use chrono::{NaiveDate, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{AccountRow, CurrencyCode, InterestRateType};
use crate::repos::{connection, settings};
use crate::schema::accounts;
use crate::state::DbPool;

/// An account's user-set interest fields: the insert values and the edit changeset.
/// `interest_accrued_through` is the daily job's cursor; it must be set whenever the rate is above
/// 0 (a DB check), and the routes set it to the day interest is turned on so nothing is backfilled.
#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = accounts, treat_none_as_null = true)]
pub struct AccountInterest {
    pub interest_rate_bps: i32,
    pub interest_rate_type: InterestRateType,
    pub interest_accrued_through: Option<NaiveDate>,
}

/// Active (non-archived) accounts, ordered by creation. Used by the API list endpoint, the
/// charge job's account picker, and the projection seed.
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

/// Count of active (non-archived) accounts. Used to forbid archiving the user's last account,
/// which would otherwise drop them into an off-book state where new rows get a NULL account.
pub async fn count_active(pool: &DbPool, user_id: Uuid) -> Result<i64, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    accounts::table
        .filter(accounts::user_id.eq(user_id))
        .filter(accounts::archived_at.is_null())
        .count()
        .get_result(&mut conn)
        .await
        .map_err(ApiError::from)
}

/// All accounts including archived ones, ordered by creation. Used by the projection seed: an
/// archived account's historical expense/income rows are still counted in projections, so its
/// initial amount must remain in the opening balance to keep the projection continuous.
pub async fn list_all_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<AccountRow>, ApiError> {
    accounts::table
        .filter(accounts::user_id.eq(user_id))
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
    exclude_from_projections: bool,
    interest: &AccountInterest,
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
                    accounts::exclude_from_projections.eq(exclude_from_projections),
                    accounts::created_at.eq(now),
                    interest,
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

/// Updates the mutable fields of an account. Currency is intentionally NOT updatable: it is fixed
/// at creation because existing expense/income rows are stored in the account's currency and are
/// summed without conversion when deriving the balance — changing it would silently corrupt the
/// balance. Only `name`, `initial_amount`, `exclude_from_projections` (when given) and the
/// interest fields can change.
pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    initial_amount: i32,
    exclude_from_projections: Option<bool>,
    interest: &AccountInterest,
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
                accounts::initial_amount.eq(initial_amount),
                exclude_from_projections.map(|exclude| accounts::exclude_from_projections.eq(exclude)),
                interest,
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

/// Moves an account's daily interest cursor from `from` to `to` and stores the uncredited carry
/// (daily job). Guarded on `from`, so it is a no-op when the cursor moved meanwhile (interest was
/// turned off and on again). Returns whether it moved.
pub async fn advance_interest(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
    carry_micros: i32,
) -> Result<bool, diesel::result::Error> {
    let updated = diesel::update(
        accounts::table
            .filter(accounts::user_id.eq(user_id))
            .filter(accounts::id.eq(id))
            .filter(accounts::interest_accrued_through.eq(from)),
    )
    .set((
        accounts::interest_accrued_through.eq(to),
        accounts::interest_carry_micros.eq(carry_micros),
    ))
    .execute(conn)
    .await?;
    Ok(updated > 0)
}
