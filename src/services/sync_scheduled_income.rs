use chrono::{Datelike, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::upsert::excluded;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::connection;
use crate::models::{IncomePayScheduleRow, IncomeSource};
use crate::schema::income;
use crate::services::pay_periods::{get_pay_dates_in_range, schedule_from_income};
use crate::state::DbPool;

const SYNC_YEARS_BACK: i32 = 1;
const SYNC_YEARS_FORWARD: i32 = 2;

fn get_sync_date_range(anchor_date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = anchor_date
        .with_year(anchor_date.year() - SYNC_YEARS_BACK)
        .unwrap_or(anchor_date);
    let today = Utc::now().date_naive();
    let end = today
        .with_year(today.year() + SYNC_YEARS_FORWARD)
        .unwrap_or(today);
    (start, end)
}

pub async fn sync_scheduled_income(
    pool: &DbPool,
    schedule: &IncomePayScheduleRow,
) -> Result<(), ApiError> {
    let user_id = schedule.user_id;
    let (start, end) = get_sync_date_range(schedule.anchor_date);
    let schedule_input = schedule_from_income(schedule);
    let pay_dates = get_pay_dates_in_range(
        &schedule_input,
        &start.format("%Y-%m-%d").to_string(),
        &end.format("%Y-%m-%d").to_string(),
    );
    let now = Utc::now();

    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            if pay_dates.is_empty() {
                diesel::delete(
                    income::table.filter(
                        income::user_id
                            .eq(user_id)
                            .and(income::schedule_id.eq(schedule.id))
                            .and(income::source.eq(IncomeSource::Scheduled)),
                    ),
                )
                .execute(conn)
                .await?;
                return Ok::<(), diesel::result::Error>(());
            }

            for pay_date_str in &pay_dates {
                let pay_date = NaiveDate::parse_from_str(pay_date_str, "%Y-%m-%d").unwrap();
                diesel::insert_into(income::table)
                    .values((
                        income::user_id.eq(user_id),
                        income::name.eq(&schedule.name),
                        income::amount.eq(schedule.amount),
                        income::currency.eq(schedule.currency),
                        income::source.eq(IncomeSource::Scheduled),
                        income::date.eq(pay_date),
                        income::schedule_id.eq(schedule.id),
                        income::created_at.eq(now),
                    ))
                    .on_conflict(diesel::pg::upsert::on_constraint(
                        "income_scheduled_schedule_date_unique",
                    ))
                    .do_update()
                    .set((
                        income::name.eq(excluded(income::name)),
                        income::amount.eq(excluded(income::amount)),
                        income::currency.eq(excluded(income::currency)),
                    ))
                    .execute(conn)
                    .await?;
            }

            let pay_date_values: Vec<NaiveDate> = pay_dates
                .iter()
                .map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap())
                .collect();

            diesel::delete(
                income::table.filter(
                    income::user_id
                        .eq(user_id)
                        .and(income::schedule_id.eq(schedule.id))
                        .and(income::source.eq(IncomeSource::Scheduled))
                        .and(income::date.ne_all(pay_date_values)),
                ),
            )
            .execute(conn)
            .await?;

            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete_scheduled_income(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
    schedule_id: Uuid,
) -> Result<(), diesel::result::Error> {
    diesel::delete(
        income::table.filter(
            income::user_id
                .eq(user_id)
                .and(income::schedule_id.eq(schedule_id))
                .and(income::source.eq(IncomeSource::Scheduled)),
        ),
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn sync_all_scheduled_income(pool: &DbPool, user_id: Uuid) -> Result<(), ApiError> {
    let schedules = crate::repos::income_schedules::list_all(pool, user_id).await?;
    for schedule in schedules {
        sync_scheduled_income(pool, &schedule).await?;
    }
    Ok(())
}
