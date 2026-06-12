use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::connection;
use crate::schema::users;
use crate::state::DbPool;

pub async fn ensure_user_exists(
    pool: &DbPool,
    user_id: Uuid,
    email: &str,
) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    diesel::insert_into(users::table)
        .values((
            users::id.eq(user_id),
            users::email.eq(Some(email)),
        ))
        .on_conflict(users::id)
        .do_nothing()
        .execute(&mut conn)
        .await?;
    Ok(())
}

pub async fn list_user_ids(pool: &DbPool) -> Result<Vec<Uuid>, ApiError> {
    let mut conn = connection::admin_connection(pool).await?;
    users::table
        .select(users::id)
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}
