use chrono::Utc;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, UserSettingsRow};
use crate::schema::user_settings;
use crate::state::DbPool;
use diesel_async::AsyncPgConnection;

pub async fn get_user_settings(pool: &DbPool, user_id: Uuid) -> Result<UserSettingsRow, ApiError> {
    let mut conn = pool.get().await?;
    if let Ok(row) = user_settings::table
        .find(user_id)
        .select(UserSettingsRow::as_select())
        .first(&mut conn)
        .await
    {
        return Ok(row);
    }

    let now = Utc::now();
    diesel::insert_into(user_settings::table)
        .values((
            user_settings::user_id.eq(user_id),
            user_settings::display_currency.eq(CurrencyCode::Usd),
            user_settings::projection_initial_free_money.eq(0),
            user_settings::updated_at.eq(now),
        ))
        .on_conflict(user_settings::user_id)
        .do_nothing()
        .execute(&mut conn)
        .await?;

    user_settings::table
        .find(user_id)
        .select(UserSettingsRow::as_select())
        .first(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn update_user_settings(
    pool: &DbPool,
    user_id: Uuid,
    display_currency: Option<CurrencyCode>,
    primary_schedule_id: Option<Option<Uuid>>,
    projection_initial_free_money: Option<i32>,
    projection_start_date: Option<Option<chrono::NaiveDate>>,
) -> Result<UserSettingsRow, ApiError> {
    get_user_settings(pool, user_id).await?;
    let mut conn = pool.get().await?;
    let now = Utc::now();

    if let Some(currency) = display_currency {
        diesel::update(user_settings::table.find(user_id))
            .set(user_settings::display_currency.eq(currency))
            .execute(&mut conn)
            .await?;
    }
    if let Some(schedule_id) = primary_schedule_id {
        diesel::update(user_settings::table.find(user_id))
            .set(user_settings::primary_schedule_id.eq(schedule_id))
            .execute(&mut conn)
            .await?;
    }
    if let Some(amount) = projection_initial_free_money {
        diesel::update(user_settings::table.find(user_id))
            .set(user_settings::projection_initial_free_money.eq(amount))
            .execute(&mut conn)
            .await?;
    }
    if let Some(start_date) = projection_start_date {
        diesel::update(user_settings::table.find(user_id))
            .set(user_settings::projection_start_date.eq(start_date))
            .execute(&mut conn)
            .await?;
    }

    diesel::update(user_settings::table.find(user_id))
        .set(user_settings::updated_at.eq(now))
        .execute(&mut conn)
        .await?;

    user_settings::table
        .find(user_id)
        .select(UserSettingsRow::as_select())
        .first(&mut conn)
        .await
        .map_err(ApiError::from)
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
