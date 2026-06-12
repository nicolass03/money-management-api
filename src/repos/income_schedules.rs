use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, IncomePayScheduleRow, PayFrequency};
use crate::schema::{income, income_pay_schedules};
use crate::repos::{connection, settings};
use crate::services::sync_scheduled_income::{delete_scheduled_income, sync_scheduled_income};
use crate::state::DbPool;

pub async fn list_all(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<IncomePayScheduleRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    income_pay_schedules::table
        .filter(income_pay_schedules::user_id.eq(user_id))
        .order(income_pay_schedules::name.asc())
        .select(IncomePayScheduleRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<IncomePayScheduleRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    income_pay_schedules::table
        .filter(income_pay_schedules::user_id.eq(user_id))
        .filter(income_pay_schedules::id.eq(id))
        .select(IncomePayScheduleRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    anchor_date: chrono::NaiveDate,
    frequency: PayFrequency,
    amount: i32,
    currency: CurrencyCode,
) -> Result<IncomePayScheduleRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    let schedule = diesel::insert_into(income_pay_schedules::table)
        .values((
            income_pay_schedules::user_id.eq(user_id),
            income_pay_schedules::name.eq(name),
            income_pay_schedules::anchor_date.eq(anchor_date),
            income_pay_schedules::frequency.eq(frequency),
            income_pay_schedules::amount.eq(amount),
            income_pay_schedules::currency.eq(currency),
            income_pay_schedules::created_at.eq(now),
            income_pay_schedules::updated_at.eq(now),
        ))
        .returning(IncomePayScheduleRow::as_returning())
        .get_result(&mut conn)
        .await?;

    sync_scheduled_income(pool, &schedule).await?;
    Ok(schedule)
}

pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    name: &str,
    anchor_date: chrono::NaiveDate,
    frequency: PayFrequency,
    amount: i32,
    currency: CurrencyCode,
) -> Result<Option<IncomePayScheduleRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    let schedule = diesel::update(
        income_pay_schedules::table
            .filter(income_pay_schedules::user_id.eq(user_id))
            .filter(income_pay_schedules::id.eq(id)),
    )
    .set((
        income_pay_schedules::name.eq(name),
        income_pay_schedules::anchor_date.eq(anchor_date),
        income_pay_schedules::frequency.eq(frequency),
        income_pay_schedules::amount.eq(amount),
        income_pay_schedules::currency.eq(currency),
        income_pay_schedules::updated_at.eq(now),
    ))
    .returning(IncomePayScheduleRow::as_returning())
    .get_result(&mut conn)
    .await
    .optional()?;

    if let Some(ref row) = schedule {
        sync_scheduled_income(pool, row).await?;
    }
    Ok(schedule)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            delete_scheduled_income(conn, user_id, id).await?;
            diesel::update(
                income::table
                    .filter(income::user_id.eq(user_id))
                    .filter(income::schedule_id.eq(id)),
            )
            .set(income::schedule_id.eq::<Option<Uuid>>(None))
            .execute(conn)
            .await?;
            settings::clear_primary_schedule(conn, user_id, id).await?;
            diesel::delete(
                income_pay_schedules::table
                    .filter(income_pay_schedules::user_id.eq(user_id))
                    .filter(income_pay_schedules::id.eq(id)),
            )
            .execute(conn)
            .await?;
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}
