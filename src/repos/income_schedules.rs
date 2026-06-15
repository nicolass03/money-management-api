use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, IncomePayScheduleRow, IncomeSource, PayFrequency};
use crate::schema::{income, income_pay_schedules};
use crate::repos::{connection, settings};
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
    find_by_id_with_conn(&mut conn, user_id, id).await
}

pub async fn find_by_id_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<IncomePayScheduleRow>, ApiError> {
    income_pay_schedules::table
        .filter(income_pay_schedules::user_id.eq(user_id))
        .filter(income_pay_schedules::id.eq(id))
        .select(IncomePayScheduleRow::as_select())
        .first(conn)
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
    conn.transaction(|conn| {
        Box::pin(async move {
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
                .get_result(conn)
                .await?;
            // No eager pre-sync: the daily cron materializes due occurrences and
            // projections cover future periods (parity with recurring expenses).
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<IncomePayScheduleRow, diesel::result::Error>(schedule)
        })
    })
    .await
    .map_err(ApiError::from)
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
    conn.transaction(|conn| {
        Box::pin(async move {
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
            .get_result(conn)
            .await
            .optional()?;
            // Already-materialized rows keep their amounts (overrides survive); future
            // occurrences pick up the new amount via projections/cron, like recurring expenses.
            if schedule.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<IncomePayScheduleRow>, diesel::result::Error>(schedule)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            // Remove materialized scheduled income (including soft-deleted tombstones)
            // for this schedule, mirroring recurring-expense delete cascading.
            diesel::delete(
                income::table.filter(
                    income::user_id
                        .eq(user_id)
                        .and(income::schedule_id.eq(id))
                        .and(income::source.eq(IncomeSource::Scheduled)),
                ),
            )
            .execute(conn)
            .await?;
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
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}
