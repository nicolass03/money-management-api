use chrono::Utc;
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{connection, income as income_repo, income_schedules, settings as settings_repo};
use crate::services::pay_periods::{get_pay_dates_in_range, schedule_from_income};
use crate::state::DbPool;

fn is_scheduled_date_unique_violation(error: &DieselError) -> bool {
    matches!(
        error,
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, info)
            if info.constraint_name().as_deref() == Some("income_scheduled_schedule_date_unique")
    )
}

/// Materializes scheduled income that falls due on `date` for one user (mirrors
/// `charge_due_expenses_for_date`). Idempotent: skips schedules that already have a
/// materialized row or soft-deleted tombstone for the date, and treats the unique-index
/// violation as an already-materialized no-op. Income is stored in the schedule's own
/// currency (projections convert per row), matching the previous eager-sync behavior.
pub async fn charge_due_income_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
) -> Result<i32, ApiError> {
    let schedules = income_schedules::list_all(pool, user_id).await?;
    let mut materialized_ids =
        income_repo::get_materialized_schedule_ids_for_date(pool, user_id, date).await?;

    let now = Utc::now();
    let due_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest("invalid date".into()))?;
    let mut created = 0;

    for schedule in schedules {
        if materialized_ids.contains(&schedule.id) {
            continue;
        }

        let input = schedule_from_income(&schedule);
        let due_dates = get_pay_dates_in_range(&input, date, date);
        if due_dates.is_empty() {
            continue;
        }

        let mut conn = connection::user_connection(pool, user_id).await?;
        let insert_result = income_repo::insert_scheduled(
            &mut conn,
            user_id,
            &schedule.name,
            schedule.amount,
            schedule.currency,
            due_date,
            schedule.id,
            schedule.account_id,
            now,
        )
        .await;

        match insert_result {
            Ok(_) => {
                materialized_ids.insert(schedule.id);
                created += 1;
            }
            Err(error) if is_scheduled_date_unique_violation(&error) => {
                materialized_ids.insert(schedule.id);
            }
            Err(error) => return Err(ApiError::from(error)),
        }
    }

    if created > 0 {
        let mut conn = connection::user_connection(pool, user_id).await?;
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
