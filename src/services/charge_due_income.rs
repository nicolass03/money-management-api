use chrono::{NaiveDate, Utc};
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{connection, income as income_repo, income_schedules, settings as settings_repo};
use crate::services::pay_periods::{occurs_on, schedule_from_income};
use crate::state::DbPool;

fn is_scheduled_date_unique_violation(error: &DieselError) -> bool {
    matches!(
        error,
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, info)
            if info.constraint_name().as_deref() == Some("income_scheduled_schedule_date_unique")
    )
}

/// Materializes scheduled income that falls due on each of `dates` for one user (mirrors
/// `charge_due_expenses`). Idempotent: skips schedules that already have a materialized row or
/// soft-deleted tombstone for the date, and treats the unique-index violation as an
/// already-materialized no-op. Income is stored in the schedule's own currency (projections
/// convert per row).
pub async fn charge_due_income(
    pool: &DbPool,
    user_id: Uuid,
    dates: &[NaiveDate],
) -> Result<i32, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let schedules = income_schedules::list_all_with_conn(&mut conn, user_id).await?;
    let now = Utc::now();
    let mut created = 0;

    for &due_date in dates {
        let materialized_ids =
            income_repo::get_materialized_schedule_ids_for_date(&mut conn, user_id, due_date)
                .await?;

        for schedule in &schedules {
            // Like recurring expenses: no occurrence before the schedule was last saved, so
            // catching up on missed days never backfills history.
            if materialized_ids.contains(&schedule.id)
                || due_date < schedule.updated_at.date_naive()
                || !occurs_on(&schedule_from_income(schedule), due_date)
            {
                continue;
            }

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
                Ok(_) => created += 1,
                Err(error) if is_scheduled_date_unique_violation(&error) => {}
                Err(error) => return Err(ApiError::from(error)),
            }
        }
    }

    if created > 0 {
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
