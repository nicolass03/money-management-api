use diesel_async::{AsyncPgConnection, SimpleAsyncConnection};
use uuid::Uuid;

use crate::error::ApiError;

/// Pins this connection to a request's user for the duration of the checkout. Sets `app.user_id`
/// (read by row-level-security policies) and clears any `app.is_admin` flag a previous checkout
/// may have left on the pooled connection — in a **single** round-trip. Interpolating the UUID is
/// injection-safe: `Uuid`'s `Display` only ever yields `[0-9a-f-]`.
pub async fn set_user_context(conn: &mut AsyncPgConnection, user_id: Uuid) -> Result<(), ApiError> {
    conn.batch_execute(&format!(
        "SELECT set_config('app.user_id', '{user_id}', false); RESET app.is_admin;"
    ))
    .await
    .map_err(ApiError::from)
}

/// Grants admin context for this checkout and clears any leftover `app.user_id`, in one round-trip.
pub async fn set_admin_context(conn: &mut AsyncPgConnection) -> Result<(), ApiError> {
    conn.batch_execute("SELECT set_config('app.is_admin', 'true', false); RESET app.user_id;")
        .await
        .map_err(ApiError::from)
}

pub async fn user_connection(
    pool: &crate::state::DbPool,
    user_id: Uuid,
) -> Result<diesel_async::pooled_connection::bb8::PooledConnection<'_, diesel_async::AsyncPgConnection>, ApiError>
{
    let mut conn = pool.get().await?;
    set_user_context(&mut conn, user_id).await?;
    Ok(conn)
}

pub async fn admin_connection(
    pool: &crate::state::DbPool,
) -> Result<diesel_async::pooled_connection::bb8::PooledConnection<'_, diesel_async::AsyncPgConnection>, ApiError>
{
    let mut conn = pool.get().await?;
    set_admin_context(&mut conn).await?;
    Ok(conn)
}

/// Checks out a connection with **no** RLS identity — both `app.user_id` and `app.is_admin` are
/// cleared. Used for queries against shared/global tables (exchange-rate snapshots, advisory
/// locks) that must not inherit a prior checkout's user context. Since fast pool recycling no
/// longer resets context, these callers establish their neutral context explicitly.
pub async fn neutral_connection(
    pool: &crate::state::DbPool,
) -> Result<diesel_async::pooled_connection::bb8::PooledConnection<'_, diesel_async::AsyncPgConnection>, ApiError>
{
    let mut conn = pool.get().await?;
    conn.batch_execute("RESET app.user_id; RESET app.is_admin;")
        .await
        .map_err(ApiError::from)?;
    Ok(conn)
}
