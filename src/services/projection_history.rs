//! Freezing past pay periods into `projection_history`.
//!
//! Closed periods are computed once (with the same [`build_projection_rows`] kernel the live path
//! uses) and persisted. The read path serves them straight from the table and only computes the
//! periods after the last frozen one live, seeded from its cumulative. Every writer — the daily
//! job, mutation paths, and the rebuild script — funnels through [`sync_history`], so the freezing
//! logic lives in exactly one place.

use chrono::{Duration, NaiveDate, Utc};
use diesel_async::AsyncConnection;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::ProjectionHistoryRow;
use crate::repos::projection_history::{self, NewProjectionHistory};
use crate::repos::{
    accounts, budgets, connection, expenses, income, income_schedules, planned_expenses,
    recurring_expenses, settings,
};
use crate::services::currency::convert_amount;
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::projections::{build_projection_rows, ProjectionInputs, ProjectionRow};
use crate::state::DbPool;
use crate::validation::today_iso;

/// Which frozen periods a [`sync_history`] call recomputes.
#[derive(Debug, Clone, Copy)]
pub enum HistorySync {
    /// Freeze newly closed periods after the last frozen one (daily job).
    Append,
    /// Re-freeze from the period containing this date onward, seeded from the last frozen period
    /// before it (a past-dated edit). Earlier periods stay untouched — including the exchange
    /// rates they were frozen with.
    From(NaiveDate),
    /// Rebuild every period from the opening balance (schedule, start date, display currency or
    /// account opening-balance changes).
    Rebuild,
}

#[derive(Debug, Clone, Copy)]
pub struct HistorySyncReport {
    pub computed: usize,
    pub inserted: usize,
}

/// A period is frozen only once its pay date is before UTC-today minus one day: by then it has
/// closed in every timezone, so a client whose local date lags UTC never sees the period it still
/// treats as current turn into a frozen past row.
fn freeze_cutoff() -> NaiveDate {
    Utc::now().date_naive() - Duration::days(1)
}

/// Loads every input the projection kernel needs. Returns `None` when the user has no primary
/// pay schedule (nothing to project) or the referenced schedule is missing.
pub async fn load_projection_inputs(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Option<ProjectionInputs>, ApiError> {
    let settings = settings::get_user_settings(pool, user_id).await?;
    let Some(schedule_id) = settings.primary_schedule_id else {
        return Ok(None);
    };

    let mut conn = connection::user_connection(pool, user_id).await?;
    let Some(primary_schedule) =
        income_schedules::find_by_id_with_conn(&mut conn, user_id, schedule_id).await?
    else {
        return Ok(None);
    };

    // Projections need every pay schedule (income can come from non-primary schedules) and
    // tombstoned rows so deleted scheduled occurrences are not re-projected.
    let rates = get_exchange_rates(pool, false).await?;
    let schedules = income_schedules::list_all_with_conn(&mut conn, user_id).await?;
    let income_all = income::list_with_deleted_with_conn(&mut conn, user_id).await?;
    let expenses = expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let recurring = recurring_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let planned = planned_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let budgets = budgets::list_with_tags_and_spent_with_conn(&mut conn, user_id).await?;

    // Opening balance = Σ every account's initial amount converted to the display currency. ALL
    // accounts (active + archived): archived accounts' historical rows still count, so their
    // initial amount must stay in the seed to keep the running balance continuous.
    let initial_free_money: i64 = accounts::list_all_with_conn(&mut conn, user_id)
        .await?
        .iter()
        .map(|account| {
            i64::from(convert_amount(
                account.initial_amount,
                account.currency,
                settings.display_currency,
                &rates,
            ))
        })
        .sum();

    Ok(Some(ProjectionInputs {
        primary_schedule,
        schedules,
        income_all,
        expenses,
        recurring,
        planned,
        budgets,
        display_currency: settings.display_currency,
        rates,
        initial_free_money,
        projection_start_date: settings.projection_start_date,
        projection_end_date: settings.projection_end_date,
    }))
}

/// Projection rows after the frozen period `base`: seeded from its cumulative and starting the day
/// after its pay date. With no base, the full projection from the opening balance. The live read
/// path and the freezer both use this, so frozen and live periods are always continuous.
pub fn build_rows_after(
    inputs: &ProjectionInputs,
    base: Option<&ProjectionHistoryRow>,
    today: &str,
) -> Vec<ProjectionRow> {
    let (seed, start) = match base {
        Some(row) => (
            row.cumulative,
            Some((row.pay_date + Duration::days(1)).format("%Y-%m-%d").to_string()),
        ),
        None => (
            inputs.initial_free_money,
            inputs.projection_start_date.map(|d| d.format("%Y-%m-%d").to_string()),
        ),
    };
    let end = inputs
        .projection_end_date
        .map(|d| d.format("%Y-%m-%d").to_string());
    build_projection_rows(inputs, seed, start.as_deref(), end.as_deref(), today)
}

/// Whether frozen rows can seed the live path. Rows frozen in another display currency can't (a
/// currency change rebuilds them; until then the read path computes everything live).
pub fn is_history_usable(history: &[ProjectionHistoryRow], inputs: &ProjectionInputs) -> bool {
    history
        .iter()
        .all(|row| row.currency == inputs.display_currency)
}

/// Brings the primary schedule's frozen periods up to date per `sync`. With `dry_run`, computes
/// and logs only.
pub async fn sync_history(
    pool: &DbPool,
    user_id: Uuid,
    sync: HistorySync,
    dry_run: bool,
) -> Result<HistorySyncReport, ApiError> {
    let Some(inputs) = load_projection_inputs(pool, user_id).await? else {
        tracing::debug!(%user_id, "no primary schedule; nothing to freeze");
        return Ok(HistorySyncReport { computed: 0, inserted: 0 });
    };
    let schedule_id = inputs.primary_schedule.id;

    let mut conn = connection::user_connection(pool, user_id).await?;
    let history = projection_history::list_for_schedule_with_conn(&mut conn, user_id, schedule_id)
        .await?;

    // How many leading frozen rows stay as they are; the rest are recomputed.
    let keep = if !is_history_usable(&history, &inputs) {
        0
    } else {
        match sync {
            HistorySync::Rebuild => 0,
            HistorySync::Append => history.len(),
            HistorySync::From(date) => history.partition_point(|row| row.pay_date < date),
        }
    };
    let base = keep.checked_sub(1).map(|index| &history[index]);

    let cutoff = freeze_cutoff();
    let rows: Vec<NewProjectionHistory> = build_rows_after(&inputs, base, &today_iso())
        .into_iter()
        .filter_map(|row| {
            let pay_date = parse_iso(&row.pay_date)?;
            (pay_date < cutoff).then_some(NewProjectionHistory {
                schedule_id,
                pay_date,
                start_date: parse_iso(&row.start_date)?,
                end_date: parse_iso(&row.end_date)?,
                income: row.income_total,
                planned_spent: row.expense_total,
                free: row.period_free,
                cumulative: row.cumulative_free,
                currency: inputs.display_currency,
            })
        })
        .collect();
    let computed = rows.len();

    let inserted = if dry_run {
        for row in &rows {
            tracing::info!(
                %user_id,
                pay_date = %row.pay_date,
                income = row.income,
                planned_spent = row.planned_spent,
                free = row.free,
                cumulative = row.cumulative,
                "projection history period (dry run)"
            );
        }
        0
    } else {
        let after = base.map(|row| row.pay_date);
        conn.transaction(|conn| {
            Box::pin(async move {
                projection_history::delete_after_with_conn(conn, user_id, schedule_id, after)
                    .await?;
                projection_history::upsert_many_with_conn(conn, user_id, &rows).await
            })
        })
        .await?
    };

    tracing::info!(%user_id, ?sync, kept = keep, computed, inserted, dry_run, "projection history synced");
    Ok(HistorySyncReport { computed, inserted })
}

/// Re-freezes history after a change dated `date`, only when that date falls in an already-frozen
/// period. The common case — a change in the current or a future period — is settled by two cheap
/// lookups without loading anything, since those periods are computed live.
pub async fn refresh_history_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: NaiveDate,
) -> Result<(), ApiError> {
    let user_settings = settings::get_user_settings(pool, user_id).await?;
    let Some(schedule_id) = user_settings.primary_schedule_id else {
        return Ok(());
    };
    let mut conn = connection::user_connection(pool, user_id).await?;
    let last_frozen =
        projection_history::last_pay_date_with_conn(&mut conn, user_id, schedule_id).await?;
    drop(conn);
    if last_frozen.is_none_or(|last| date > last) {
        return Ok(());
    }

    tracing::info!(%user_id, %date, "past-dated change; re-freezing projection history from its period");
    sync_history(pool, user_id, HistorySync::From(date), false).await?;
    Ok(())
}

/// Drops all of the user's frozen periods (see `delete_all_for_user_with_conn`).
pub async fn clear_history(pool: &DbPool, user_id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    projection_history::delete_all_for_user_with_conn(&mut conn, user_id).await?;
    Ok(())
}

fn parse_iso(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CurrencyCode, IncomePayScheduleRow, PayFrequency};
    use crate::services::currency::ExchangeRates;
    use std::collections::HashMap;

    fn inputs() -> ProjectionInputs {
        let schedule = IncomePayScheduleRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "salary".to_string(),
            anchor_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            frequency: PayFrequency::Monthly,
            amount: 100_000,
            currency: CurrencyCode::Usd,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            account_id: None,
        };
        ProjectionInputs {
            schedules: vec![schedule.clone()],
            primary_schedule: schedule,
            income_all: Vec::new(),
            expenses: Vec::new(),
            recurring: Vec::new(),
            planned: Vec::new(),
            budgets: Vec::new(),
            display_currency: CurrencyCode::Usd,
            rates: ExchangeRates {
                base: "usd".to_string(),
                rates: HashMap::new(),
                fetched_at: "2026-06-01".to_string(),
            },
            initial_free_money: 5_000,
            projection_start_date: NaiveDate::from_ymd_opt(2026, 1, 1),
            projection_end_date: None,
        }
    }

    #[test]
    fn freezes_only_closed_periods_with_carried_balance() {
        let inputs = inputs();
        let rows: Vec<ProjectionRow> = build_rows_after(&inputs, None, "2026-06-01")
            .into_iter()
            .filter(|row| row.is_past)
            .collect();

        // Pay dates strictly before today: 2026-01-15 .. 2026-05-15 (five closed periods). The
        // 2026-06-15 period is current and must not be frozen.
        let pay_dates: Vec<String> = rows.iter().map(|row| row.pay_date.clone()).collect();
        assert_eq!(
            pay_dates,
            vec![
                "2026-01-15",
                "2026-02-15",
                "2026-03-15",
                "2026-04-15",
                "2026-05-15",
            ]
        );

        // No income or expenses, so every period is flat and the opening balance is carried through.
        // The opening (partial) period's "free" shows the starting balance itself.
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.income_total, 0);
            assert_eq!(row.expense_total, 0);
            assert_eq!(row.period_free, if index == 0 { 5_000 } else { 0 });
            assert_eq!(row.cumulative_free, 5_000);
        }
    }
}
