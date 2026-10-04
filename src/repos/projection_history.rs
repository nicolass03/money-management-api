use chrono::NaiveDate;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, ProjectionHistoryRow};
use crate::schema::projection_history;

/// A frozen projection period ready to persist. `user_id` is supplied at write time so the
/// caller's RLS context and the row always agree.
#[derive(Debug, Clone)]
pub struct NewProjectionHistory {
    pub schedule_id: Uuid,
    pub pay_date: NaiveDate,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub income: i64,
    pub planned_spent: i64,
    pub free: i64,
    pub cumulative: i64,
    pub currency: CurrencyCode,
}

/// All frozen periods for a schedule, oldest first. Reads use this to serve past periods and to
/// seed the live current period from the latest cumulative.
pub async fn list_for_schedule_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    schedule_id: Uuid,
) -> Result<Vec<ProjectionHistoryRow>, ApiError> {
    projection_history::table
        .filter(projection_history::user_id.eq(user_id))
        .filter(projection_history::schedule_id.eq(schedule_id))
        .order(projection_history::pay_date.asc())
        .select(ProjectionHistoryRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

/// Appends frozen periods, skipping any period already present for the schedule. Idempotent, so
/// the daily job and the init script can run repeatedly without duplicating rows. Returns the
/// number of rows actually inserted.
pub async fn upsert_many_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    rows: &[NewProjectionHistory],
) -> Result<usize, ApiError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let values: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                projection_history::user_id.eq(user_id),
                projection_history::schedule_id.eq(row.schedule_id),
                projection_history::pay_date.eq(row.pay_date),
                projection_history::start_date.eq(row.start_date),
                projection_history::end_date.eq(row.end_date),
                projection_history::income.eq(row.income),
                projection_history::planned_spent.eq(row.planned_spent),
                projection_history::free.eq(row.free),
                projection_history::cumulative.eq(row.cumulative),
                projection_history::currency.eq(row.currency),
            )
        })
        .collect();

    diesel::insert_into(projection_history::table)
        .values(values)
        .on_conflict((
            projection_history::user_id,
            projection_history::schedule_id,
            projection_history::pay_date,
        ))
        .do_nothing()
        .execute(conn)
        .await
        .map_err(ApiError::from)
}

/// Drops a schedule's frozen periods closing after `after` (all of them when `None`), so they can be
/// re-frozen from the last kept row onward.
pub async fn delete_after_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    schedule_id: Uuid,
    after: Option<NaiveDate>,
) -> Result<usize, ApiError> {
    let mut query = diesel::delete(
        projection_history::table
            .filter(projection_history::user_id.eq(user_id))
            .filter(projection_history::schedule_id.eq(schedule_id)),
    )
    .into_boxed();
    if let Some(after) = after {
        query = query.filter(projection_history::pay_date.gt(after));
    }
    query.execute(conn).await.map_err(ApiError::from)
}

/// Pay date of the latest frozen period for a schedule, if any.
pub async fn last_pay_date_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    schedule_id: Uuid,
) -> Result<Option<NaiveDate>, ApiError> {
    projection_history::table
        .filter(projection_history::user_id.eq(user_id))
        .filter(projection_history::schedule_id.eq(schedule_id))
        .select(diesel::dsl::max(projection_history::pay_date))
        .first(conn)
        .await
        .map_err(ApiError::from)
}

/// Drops every frozen period for the user (all schedules). Last-resort recovery when a re-freeze
/// fails: with no frozen rows, reads fall back to a full live computation until the daily job
/// freezes them again, rather than serving stale aggregates.
pub async fn delete_all_for_user_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<usize, ApiError> {
    diesel::delete(projection_history::table.filter(projection_history::user_id.eq(user_id)))
        .execute(conn)
        .await
        .map_err(ApiError::from)
}
