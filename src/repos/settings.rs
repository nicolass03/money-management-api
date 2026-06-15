use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, UserSettingsRow};
use crate::repos::connection;
use crate::schema::user_settings;
use crate::state::DbPool;
use diesel_async::AsyncPgConnection;

pub async fn get_user_settings(pool: &DbPool, user_id: Uuid) -> Result<UserSettingsRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    get_user_settings_with_conn(&mut conn, user_id).await
}

pub async fn get_user_settings_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<UserSettingsRow, ApiError> {
    if let Ok(row) = user_settings::table
        .find(user_id)
        .select(UserSettingsRow::as_select())
        .first(conn)
        .await
    {
        return Ok(row);
    }

    let now = Utc::now();
    diesel::insert_into(user_settings::table)
        .values((
            user_settings::user_id.eq(user_id),
            user_settings::display_currency.eq(CurrencyCode::Usd),
            user_settings::language.eq("en"),
            user_settings::projection_initial_free_money.eq(0),
            user_settings::updated_at.eq(now),
        ))
        .on_conflict(user_settings::user_id)
        .do_nothing()
        .execute(conn)
        .await?;

    user_settings::table
        .find(user_id)
        .select(UserSettingsRow::as_select())
        .first(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn update_user_settings(
    pool: &DbPool,
    user_id: Uuid,
    display_currency: Option<CurrencyCode>,
    language: Option<String>,
    primary_schedule_id: Option<Option<Uuid>>,
    projection_initial_free_money: Option<i32>,
    projection_start_date: Option<Option<chrono::NaiveDate>>,
    extra_spent_limit: Option<Option<i32>>,
) -> Result<UserSettingsRow, ApiError> {
    get_user_settings(pool, user_id).await?;
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();

    conn.transaction(|conn| {
        Box::pin(async move {
            if let Some(currency) = display_currency {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::display_currency.eq(currency))
                    .execute(conn)
                    .await?;
            }
            if let Some(language) = language {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::language.eq(language))
                    .execute(conn)
                    .await?;
            }
            if let Some(schedule_id) = primary_schedule_id {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::primary_schedule_id.eq(schedule_id))
                    .execute(conn)
                    .await?;
            }
            if let Some(amount) = projection_initial_free_money {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::projection_initial_free_money.eq(amount))
                    .execute(conn)
                    .await?;
            }
            if let Some(start_date) = projection_start_date {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::projection_start_date.eq(start_date))
                    .execute(conn)
                    .await?;
            }
            if let Some(limit) = extra_spent_limit {
                diesel::update(user_settings::table.find(user_id))
                    .set(user_settings::extra_spent_limit.eq(limit))
                    .execute(conn)
                    .await?;
            }

            diesel::update(user_settings::table.find(user_id))
                .set(user_settings::updated_at.eq(now))
                .execute(conn)
                .await?;

            bump_cache_revision(conn, user_id).await?;

            user_settings::table
                .find(user_id)
                .select(UserSettingsRow::as_select())
                .first(conn)
                .await
        })
    })
    .await
    .map_err(ApiError::from)
}

/// Increments `cache_revision` so revision-keyed caches miss after writes.
pub async fn bump_cache_revision(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<(), diesel::result::Error> {
    let now = Utc::now();
    diesel::insert_into(user_settings::table)
        .values((
            user_settings::user_id.eq(user_id),
            user_settings::display_currency.eq(CurrencyCode::Usd),
            user_settings::language.eq("en"),
            user_settings::projection_initial_free_money.eq(0),
            user_settings::updated_at.eq(now),
            user_settings::cache_revision.eq(1_i64),
        ))
        .on_conflict(user_settings::user_id)
        .do_update()
        .set((
            user_settings::cache_revision
                .eq(user_settings::cache_revision + 1),
            user_settings::updated_at.eq(now),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn clear_primary_schedule(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    schedule_id: Uuid,
) -> Result<(), diesel::result::Error> {
    diesel::update(
        user_settings::table
            .filter(user_settings::user_id.eq(user_id))
            .filter(user_settings::primary_schedule_id.eq(schedule_id)),
    )
    .set(user_settings::primary_schedule_id.eq::<Option<Uuid>>(None))
    .execute(conn)
    .await?;
    Ok(())
}
