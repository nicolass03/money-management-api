use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use diesel::sql_query;
use diesel::sql_types::BigInt;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::cache::{InvalidationScope, UserDataCache};
use crate::config::Config;
use crate::error::ApiError;
use crate::repos::{connection, users};
use crate::services::charge_due_expenses::charge_due_expenses_for_date;
use crate::services::charge_due_income::charge_due_income_for_date;
use crate::services::projection_history::ensure_history;
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
        // Every per-user step is isolated: one user's failure (transient DB error, bad row) must
        // not abort the whole batch and skip everyone else. Each step self-heals on the next run.
        let expenses_created = match charge_due_expenses_for_date(pool, user_id, &date).await {
            Ok(count) => {
                if count > 0 {
                    if let Some(cache) = cache {
                        cache.invalidate(InvalidationScope::ExpenseChange, user_id).await;
                    }
                }
                count
            }
            Err(error) => {
                tracing::error!(%user_id, %error, "charge due expenses failed");
                0
            }
        };

        let income_created = match charge_due_income_for_date(pool, user_id, &date).await {
            Ok(count) => {
                if count > 0 {
                    if let Some(cache) = cache {
                        cache.invalidate(InvalidationScope::IncomeChange, user_id).await;
                    }
                }
                count
            }
            Err(error) => {
                tracing::error!(%user_id, %error, "charge due income failed");
                0
            }
        };

        // Cancellation-reminder rows drive the web banners; iOS schedules its own local
        // notifications. They are not server-cached, so no invalidation is needed here.
        let reminders_created =
            match generate_subscription_reminders_for_date(pool, user_id, &date).await {
                Ok(count) => count,
                Err(error) => {
                    tracing::error!(%user_id, %error, "subscription reminders failed");
                    0
                }
            };

        // Freeze any period that has now closed. Runs after materialization so a period closing
        // today already has its due income/expenses materialized before it is frozen.
        let history_created = match ensure_history(pool, user_id).await {
            Ok(report) => report.inserted as i32,
            Err(error) => {
                tracing::error!(%user_id, %error, "projection history freeze failed");
                0
            }
        };

        created += expenses_created + income_created + reminders_created + history_created;
    }
    Ok((date, created))
}

#[derive(diesel::QueryableByName)]
struct AdvisoryLockRow {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    acquired: bool,
}

/// A pooled connection whose session holds the daily-job advisory lock.
type LockConn<'a> = diesel_async::pooled_connection::bb8::PooledConnection<'a, AsyncPgConnection>;

/// Tries to take the advisory lock and, on success, returns the connection that holds it.
///
/// `pg_try_advisory_lock` is a **session** lock: it is owned by the connection (session) that took
/// it and can only be released by that same session. We therefore keep the connection alive for the
/// whole job and release on it directly — returning it to the pool between acquire and release
/// would let `pg_advisory_unlock` run on a different pooled session, silently fail, and leak the
/// lock (blocking every subsequent run until that connection's session ends).
async fn acquire_lock(pool: &DbPool) -> Result<Option<LockConn<'_>>, ApiError> {
    let mut conn = connection::neutral_connection(pool).await?;
    let row: AdvisoryLockRow = sql_query("SELECT pg_try_advisory_lock($1) AS acquired")
        .bind::<BigInt, _>(DAILY_EXPENSES_LOCK_KEY)
        .get_result(&mut conn)
        .await
        .map_err(ApiError::from)?;
    Ok(row.acquired.then_some(conn))
}

async fn release_lock(conn: &mut LockConn<'_>) {
    let _: Result<AdvisoryLockRow, _> = sql_query("SELECT pg_advisory_unlock($1) AS acquired")
        .bind::<BigInt, _>(DAILY_EXPENSES_LOCK_KEY)
        .get_result(conn)
        .await;
}

/// Time until the next `hour`:00. `hour` is interpreted in the **server's** local timezone
/// (`Local`), which on Railway is UTC — so `daily_expenses_hour` is effectively a UTC hour. Kept
/// simple deliberately: an approximate daily tick is fine, and the advisory lock guards against a
/// DST-induced double fire.
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

            match acquire_lock(&pool).await {
                Ok(Some(mut lock_conn)) => {
                    tracing::info!("daily expense scheduler acquired lock");
                    match run_daily_expenses(&pool, Some(cache.as_ref())).await {
                        Ok((date, created)) => {
                            tracing::info!(%date, created, "daily expense job completed");
                        }
                        Err(error) => {
                            tracing::error!(%error, "daily expense job failed");
                        }
                    }
                    // Released on the same session that acquired it; the connection then returns to
                    // the pool with the lock cleared.
                    release_lock(&mut lock_conn).await;
                }
                Ok(None) => {
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
