use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::{AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};
use uuid::Uuid;

use crate::error::ApiError;

pub async fn reset_rls_context(conn: &mut AsyncPgConnection) -> Result<(), ApiError> {
    conn.batch_execute("RESET app.user_id; RESET app.is_admin;")
        .await
        .map_err(ApiError::from)
}

pub async fn set_user_context(conn: &mut AsyncPgConnection, user_id: Uuid) -> Result<(), ApiError> {
    sql_query("SELECT set_config('app.user_id', $1, false)")
        .bind::<Text, _>(user_id.to_string())
        .execute(conn)
        .await
        .map_err(ApiError::from)?;
    Ok(())
}

pub async fn set_admin_context(conn: &mut AsyncPgConnection) -> Result<(), ApiError> {
    sql_query("SELECT set_config('app.is_admin', 'true', false)")
        .execute(conn)
        .await
        .map_err(ApiError::from)?;
    Ok(())
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
