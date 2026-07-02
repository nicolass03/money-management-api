//! Freezing past pay periods into `projection_history`.
//!
//! Past periods never change, so their aggregates are computed once (with the same
//! [`build_projection_rows`] kernel the live path uses) and persisted. The read path then serves
//! them straight from the table and only computes the current + future periods live. The daily
//! job, the settings/schedule mutation paths, and the init script all funnel through
//! [`sync_history`] so the freezing logic lives in exactly one place.

use chrono::NaiveDate;
use diesel_async::AsyncConnection;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow,
};
use crate::repos::projection_history::{self, NewProjectionHistory};
use crate::repos::{
    accounts, budgets, connection, expenses, income, income_schedules, planned_expenses,
    recurring_expenses, settings,
};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::pay_periods::{get_period_containing, schedule_from_income};
use crate::services::projections::build_projection_rows;
use crate::state::DbPool;
use crate::validation::today_iso;

/// Everything [`build_projection_rows`] needs for one user, loaded once.
pub struct ProjectionInputs {
    pub primary_schedule: IncomePayScheduleRow,
    pub schedules: Vec<IncomePayScheduleRow>,
    pub income_all: Vec<IncomeRow>,
    pub expenses: Vec<(ExpenseRow, Vec<String>)>,
    pub recurring: Vec<(RecurringExpenseRow, Vec<String>)>,
    pub planned: Vec<(PlannedExpenseRow, Vec<String>)>,
    pub budgets: Vec<(BudgetRow, Vec<String>, i32)>,
    pub display_currency: CurrencyCode,
    pub rates: ExchangeRates,
    pub initial_free_money: i32,
    pub projection_start_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOptions {
    /// Delete the primary schedule's existing rows before writing. Used when the schedule's
    /// periodicity/boundaries or the display currency changed, so stale values never linger.
    pub replace: bool,
    /// Compute and log only; never touch the database. Used by the init script for verification.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct HistorySyncReport {
    pub computed: usize,
    pub inserted: usize,
}

/// Loads every input [`build_projection_rows`] needs. Returns `None` when the user has no primary
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
    let schedules = income_schedules::list_all(pool, user_id).await?;
    let income_all = income::list_with_deleted_with_conn(&mut conn, user_id).await?;
    let expenses = expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let recurring = recurring_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let planned = planned_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
    let budgets = budgets::list_with_tags_and_spent_with_conn(&mut conn, user_id).await?;

    // Opening balance = projection setting (display currency) + every account's initial amount
    // converted into the display currency (mirrors the loader's projection seed).
    let accounts_list = accounts::list_active_with_conn(&mut conn, user_id).await?;
    let accounts_initial: i32 = accounts_list
        .iter()
        .map(|account| {
            convert_amount(
                account.initial_amount,
                account.currency,
                settings.display_currency,
                &rates,
            )
        })
        .sum();
    let initial_free_money = settings.projection_initial_free_money + accounts_initial;

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
    }))
}

/// Computes the frozen rows for the user's primary schedule: run the full projection and keep only
/// the periods that have already closed (`is_past`, i.e. `pay_date < today`).
pub fn compute_history_rows(inputs: &ProjectionInputs, today: &str) -> Vec<NewProjectionHistory> {
    let projection_start = inputs
        .projection_start_date
        .map(|date| date.format("%Y-%m-%d").to_string());

    let rows = build_projection_rows(
        &inputs.primary_schedule,
        &inputs.schedules,
        &inputs.income_all,
        &inputs.expenses,
        &inputs.recurring,
        &inputs.planned,
        &inputs.budgets,
        inputs.display_currency,
        &inputs.rates,
        inputs.initial_free_money,
        projection_start.as_deref(),
        today,
    );

    rows.into_iter()
        .filter(|row| row.is_past)
        .filter_map(|row| {
            Some(NewProjectionHistory {
                schedule_id: inputs.primary_schedule.id,
                pay_date: parse_iso(&row.pay_date)?,
                start_date: parse_iso(&row.start_date)?,
                end_date: parse_iso(&row.end_date)?,
                income: row.income_total,
                planned_spent: row.expense_total,
                free: row.period_free,
                cumulative: row.cumulative_free,
                currency: inputs.display_currency,
            })
        })
        .collect()
}

/// Freezes all closed periods for a user's primary schedule. Idempotent (`ON CONFLICT DO
/// NOTHING`), so the daily job self-heals any missed day. With `replace`, the schedule's rows are
/// first deleted (for a periodicity/currency change); with `dry_run`, nothing is written.
pub async fn sync_history(
    pool: &DbPool,
    user_id: Uuid,
    opts: SyncOptions,
) -> Result<HistorySyncReport, ApiError> {
    let Some(inputs) = load_projection_inputs(pool, user_id).await? else {
        tracing::debug!(%user_id, "no primary schedule; nothing to freeze");
        return Ok(HistorySyncReport { computed: 0, inserted: 0 });
    };
    write_history(pool, user_id, &inputs, opts).await
}

/// Freezes the closed periods described by pre-loaded `inputs`. Split from [`sync_history`] so
/// callers that already hold the inputs (or only need them for a cheap check) don't reload.
async fn write_history(
    pool: &DbPool,
    user_id: Uuid,
    inputs: &ProjectionInputs,
    opts: SyncOptions,
) -> Result<HistorySyncReport, ApiError> {
    let today = today_iso();
    let rows = compute_history_rows(inputs, &today);
    let computed = rows.len();

    for row in &rows {
        tracing::info!(
            %user_id,
            schedule_id = %inputs.primary_schedule.id,
            pay_date = %row.pay_date,
            start_date = %row.start_date,
            end_date = %row.end_date,
            income = row.income,
            planned_spent = row.planned_spent,
            free = row.free,
            cumulative = row.cumulative,
            currency = row.currency.as_str(),
            dry_run = opts.dry_run,
            "projection history period"
        );
    }

    let inserted = if opts.dry_run {
        0
    } else {
        let schedule_id = inputs.primary_schedule.id;
        let mut conn = connection::user_connection(pool, user_id).await?;
        if opts.replace {
            conn.transaction(|conn| {
                Box::pin(async move {
                    projection_history::delete_for_schedule_with_conn(conn, user_id, schedule_id)
                        .await?;
                    projection_history::upsert_many_with_conn(conn, user_id, &rows).await
                })
            })
            .await?
        } else {
            projection_history::upsert_many_with_conn(&mut conn, user_id, &rows).await?
        }
    };

    tracing::info!(
        %user_id,
        computed,
        inserted,
        dry_run = opts.dry_run,
        replace = opts.replace,
        "projection history synced"
    );

    Ok(HistorySyncReport { computed, inserted })
}

/// Appends any newly-closed periods for the primary schedule. Used by the daily job.
pub async fn ensure_history(pool: &DbPool, user_id: Uuid) -> Result<HistorySyncReport, ApiError> {
    sync_history(pool, user_id, SyncOptions::default()).await
}

/// Rebuilds the primary schedule's frozen rows from scratch. Used after a schedule switch/edit or
/// a display-currency change, where existing values are no longer valid.
pub async fn reinitialize_history(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<HistorySyncReport, ApiError> {
    sync_history(pool, user_id, SyncOptions { replace: true, dry_run: false }).await
}

/// Re-freezes past history when a mutation lands in an already-closed period, so frozen
/// aggregates stay in sync with edits to past-dated data. The common case — a change in the
/// current or a future period — is detected with two cheap lookups (settings + primary schedule)
/// and returns without touching the history table, since those periods are computed live.
pub async fn refresh_history_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
) -> Result<(), ApiError> {
    let user_settings = settings::get_user_settings(pool, user_id).await?;
    let Some(schedule_id) = user_settings.primary_schedule_id else {
        return Ok(());
    };
    let mut conn = connection::user_connection(pool, user_id).await?;
    let Some(schedule_row) =
        income_schedules::find_by_id_with_conn(&mut conn, user_id, schedule_id).await?
    else {
        return Ok(());
    };
    drop(conn);

    let today = today_iso();
    let current = get_period_containing(&schedule_from_income(&schedule_row), &today);
    if date >= current.start_date.as_str() {
        // Current or future period — nothing is frozen there, so the live path already reflects it.
        return Ok(());
    }

    tracing::info!(%user_id, date, "past-dated change; rebuilding frozen projection history");
    reinitialize_history(pool, user_id).await?;
    Ok(())
}

fn parse_iso(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PayFrequency;
    use chrono::Utc;
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
        }
    }

    #[test]
    fn freezes_only_closed_periods_with_carried_balance() {
        let inputs = inputs();
        let rows = compute_history_rows(&inputs, "2026-06-01");

        // Pay dates strictly before today: 2026-01-15 .. 2026-05-15 (five closed periods). The
        // 2026-06-15 period is current and must not be frozen.
        let pay_dates: Vec<String> = rows
            .iter()
            .map(|row| row.pay_date.format("%Y-%m-%d").to_string())
            .collect();
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
        for row in &rows {
            assert_eq!(row.income, 0);
            assert_eq!(row.planned_spent, 0);
            assert_eq!(row.free, 0);
            assert_eq!(row.cumulative, 5_000);
            assert_eq!(row.schedule_id, inputs.primary_schedule.id);
            assert_eq!(row.currency, CurrencyCode::Usd);
        }
    }
}
