use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use diesel::sql_query;
use diesel::sql_types::BigInt;
use diesel_async::RunQueryDsl;

use crate::cache::{InvalidationScope, UserDataCache};
use crate::config::Config;
use crate::error::ApiError;
use crate::repos::{connection, users};
use crate::services::charge_due_expenses::charge_due_expenses_for_date;
use crate::services::charge_due_income::charge_due_income_for_date;
use crate::services::subscription_reminders::generate_subscription_reminders_for_date;
use crate::state::DbPool;
use crate::validation::today_iso;

/// Advisory lock key for the daily materialization job (arbitrary stable id).
const DAILY_EXPENSES_LOCK_KEY: i64 = 8_451_903_221;

/// Materializes both due recurring expenses and due scheduled income for today.
/// Income mirrors the recurring-expense flow: due occurrences become actual rows on
/// their pay date, and each resource invalidates its own cache scope independently.
pub async fn run_daily_expenses(
    pool: &DbPool,
    cache: Option<&UserDataCache>,
) -> Result<(String, i32), ApiError> {
    let date = today_iso();
    let user_ids = users::list_user_ids(pool).await?;
    let mut created = 0;
    for user_id in user_ids {
        let expenses_created = charge_due_expenses_for_date(pool, user_id, &date).await?;
        if expenses_created > 0 {
            if let Some(cache) = cache {
                cache.invalidate(InvalidationScope::ExpenseChange, user_id).await;
            }
        }

        let income_created = charge_due_income_for_date(pool, user_id, &date).await?;
        if income_created > 0 {
            if let Some(cache) = cache {
                cache.invalidate(InvalidationScope::IncomeChange, user_id).await;
            }
        }

        // Cancellation-reminder rows drive the web banners; iOS schedules its own local
        // notifications. They are not server-cached, so no invalidation is needed here.
        let reminders_created =
            generate_subscription_reminders_for_date(pool, user_id, &date).await?;

        created += expenses_created + income_created + reminders_created;
    }
    Ok((date, created))
}

#[derive(diesel::QueryableByName)]
struct AdvisoryLockRow {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    acquired: bool,
}

async fn try_acquire_lock(pool: &DbPool) -> Result<bool, ApiError> {
    let mut conn = connection::neutral_connection(pool).await?;
    let row: AdvisoryLockRow = sql_query("SELECT pg_try_advisory_lock($1) AS acquired")
        .bind::<BigInt, _>(DAILY_EXPENSES_LOCK_KEY)
        .get_result(&mut conn)
        .await
        .map_err(ApiError::from)?;
    Ok(row.acquired)
}

async fn release_lock(pool: &DbPool) {
    if let Ok(mut conn) = connection::neutral_connection(pool).await {
        let _: Result<AdvisoryLockRow, _> = sql_query("SELECT pg_advisory_unlock($1) AS acquired")
            .bind::<BigInt, _>(DAILY_EXPENSES_LOCK_KEY)
            .get_result(&mut conn)
            .await;
    }
}

fn duration_until_next_run(hour: u8) -> Duration {
    let now = Local::now();
    let target_hour = hour.min(23);
    let mut next = now
        .date_naive()
        .and_hms_opt(u32::from(target_hour), 0, 0)
        .unwrap();
    if now.naive_local() >= next {
        next += chrono::Duration::days(1);
    }
    let wait = next - now.naive_local();
    Duration::from_secs(wait.num_seconds().max(0) as u64)
}

pub fn spawn_scheduler(pool: DbPool, cache: Arc<UserDataCache>, config: &Config) {
    if !config.enable_internal_cron {
        tracing::info!("internal daily expense scheduler disabled");
        return;
    }

    let hour = config.daily_expenses_hour;
    tokio::spawn(async move {
        loop {
            let wait = duration_until_next_run(hour);
            tracing::debug!(?wait, hour, "daily expense scheduler sleeping");
            tokio::time::sleep(wait).await;

            match try_acquire_lock(&pool).await {
                Ok(true) => {
                    tracing::info!("daily expense scheduler acquired lock");
                    match run_daily_expenses(&pool, Some(cache.as_ref())).await {
                        Ok((date, created)) => {
                            tracing::info!(%date, created, "daily expense job completed");
                        }
                        Err(error) => {
                            tracing::error!(%error, "daily expense job failed");
                        }
                    }
                    release_lock(&pool).await;
                }
                Ok(false) => {
                    tracing::debug!("daily expense scheduler skipped; another instance holds lock");
                }
                Err(error) => {
                    tracing::error!(%error, "daily expense scheduler lock failed");
                }
            }
        }
    });

    tracing::info!(hour, "internal daily expense scheduler started");
}
