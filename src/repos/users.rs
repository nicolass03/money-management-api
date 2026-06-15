use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::Bool;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::connection;
use crate::schema::users;
use crate::state::DbPool;

#[derive(diesel::QueryableByName)]
struct PasswordCheckRow {
    #[diesel(sql_type = Bool)]
    has_password: bool,
}

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

pub async fn is_onboarding_complete(pool: &DbPool, user_id: Uuid) -> Result<bool, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let completed_at: Option<DateTime<Utc>> = users::table
        .find(user_id)
        .select(users::onboarding_completed_at)
        .first(&mut conn)
        .await?;
    Ok(completed_at.is_some())
}

/// True when Supabase auth.users has a non-empty encrypted_password (invite accepted + password set).
pub async fn has_auth_password(pool: &DbPool, user_id: Uuid) -> Result<bool, ApiError> {
    let mut conn = connection::admin_connection(pool).await?;
    let row = sql_query(
        "SELECT (encrypted_password IS NOT NULL AND length(encrypted_password) > 0) AS has_password
         FROM auth.users
         WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(user_id)
    .get_result::<PasswordCheckRow>(&mut conn)
    .await
    .optional()?;

    Ok(row.map(|value| value.has_password).unwrap_or(false))
}

pub async fn complete_onboarding(pool: &DbPool, user_id: Uuid) -> Result<(), ApiError> {
    if !has_auth_password(pool, user_id).await? {
        return Err(ApiError::BadRequest(
            "password must be set before completing onboarding".into(),
        ));
    }

    let mut conn = connection::user_connection(pool, user_id).await?;
    diesel::update(users::table.find(user_id))
        .set(users::onboarding_completed_at.eq(Some(Utc::now())))
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
