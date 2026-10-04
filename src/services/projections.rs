use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow,
};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::expense_period::{
    build_expense_period_materialized, get_expense_items_in_period, to_budget_with_tags,
    to_expense_with_tags, to_planned_with_tags, to_recurring_with_tags, ExpenseItemSort,
    GetExpenseItemsOptions,
};
use crate::services::pay_periods::{
    get_pay_dates_in_range, get_projection_periods, is_date_in_period, schedule_from_income,
    PayPeriod,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionExpenseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_expense_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_total: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_spent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_budget_summary: Option<bool>,
    pub name: String,
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_date: Option<String>,
    pub amount: i32,
    pub currency: CurrencyCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_amount: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_currency: Option<CurrencyCode>,
    pub converted_amount: i32,
    pub tags: Vec<String>,
    pub is_subscription: bool,
    pub projected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// Sort expense rows for list display. Projections use chronological date; the expenses tab lists
/// recorded expenses newest date first (entry time only breaks ties within a day, so back-dated
/// entries land on their own date), followed by projected/summary rows.
pub(crate) fn sort_projection_expense_items(
    items: &mut [ProjectionExpenseItem],
    sort: ExpenseItemSort,
) {
    match sort {
        ExpenseItemSort::DateDesc => items.sort_by(|a, b| match (&a.created_at, &b.created_at) {
            (Some(a_ts), Some(b_ts)) => b.date.cmp(&a.date).then_with(|| b_ts.cmp(a_ts)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.date.cmp(&a.date),
        }),
        ExpenseItemSort::DateAsc => items.sort_by(|a, b| a.date.cmp(&b.date)),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionRow {
    pub pay_date: String,
    pub start_date: String,
    pub end_date: String,
    pub income_total: i64,
    pub expense_total: i64,
    pub period_free: i64,
    pub cumulative_free: i64,
    pub expense_items: Vec<ProjectionExpenseItem>,
    pub is_past: bool,
}

/// Everything the projection kernel needs for one user, loaded once (see
/// `projection_history::load_projection_inputs`). Both the live read path and the history freezer
/// build from this, so they can never disagree on inputs such as the opening balance.
pub struct ProjectionInputs {
    pub primary_schedule: IncomePayScheduleRow,
    pub schedules: Vec<IncomePayScheduleRow>,
    /// Includes soft-deleted tombstones (they block re-projecting a deleted occurrence).
    pub income_all: Vec<IncomeRow>,
    pub expenses: Vec<(ExpenseRow, Vec<String>)>,
    pub recurring: Vec<(RecurringExpenseRow, Vec<String>)>,
    pub planned: Vec<(PlannedExpenseRow, Vec<String>)>,
    pub budgets: Vec<(BudgetRow, Vec<String>, i32)>,
    pub display_currency: CurrencyCode,
    pub rates: ExchangeRates,
    /// Opening balance: Σ every account's (active + archived) initial amount in display currency.
    pub initial_free_money: i64,
    pub projection_start_date: Option<NaiveDate>,
    pub projection_end_date: Option<NaiveDate>,
}

fn is_on_or_after_start_date(date: &str, start_date: Option<&str>) -> bool {
    start_date.is_none_or(|start| date >= start)
}

fn effective_period_start(period: &PayPeriod, projection_start_date: Option<&str>) -> String {
    if let Some(start) = projection_start_date {
        if start > period.start_date.as_str() && start <= period.end_date.as_str() {
            return start.to_string();
        }
    }
    period.start_date.clone()
}

/// Persisted (non-deleted) income dated inside `period`, in display currency.
fn sum_income_in_period(
    entries: &[IncomeRow],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> i64 {
    entries
        .iter()
        .filter(|entry| entry.deleted_at.is_none())
        .filter(|entry| is_date_in_period(&entry.date.format("%Y-%m-%d").to_string(), period))
        .map(|entry| i64::from(convert_amount(entry.amount, entry.currency, display_currency, rates)))
        .sum()
}

/// `(schedule_id, date)` slots that already have a materialized income row — including
/// soft-deleted tombstones — so a future occurrence is projected at most once and a
/// deleted occurrence is never resurrected.
fn scheduled_income_keys(entries: &[IncomeRow]) -> HashSet<(Uuid, NaiveDate)> {
    entries
        .iter()
        .filter_map(|entry| entry.schedule_id.map(|sid| (sid, entry.date)))
        .collect()
}

/// Projected scheduled income for a period: future pay-date occurrences (on or after
/// `today`) from every pay schedule that have not yet been materialized or tombstoned.
/// Past/current occurrences come from persisted rows via `sum_income_in_period`, mirroring
/// how expenses treat past periods as actual and future periods as projected.
fn projected_income_in_period(
    schedules: &[IncomePayScheduleRow],
    period: &PayPeriod,
    materialized: &HashSet<(Uuid, NaiveDate)>,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    today: &str,
) -> i64 {
    let mut total = 0;
    for schedule in schedules {
        let input = schedule_from_income(schedule);
        for date_str in get_pay_dates_in_range(&input, &period.start_date, &period.end_date) {
            if date_str.as_str() < today {
                continue;
            }
            let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") else {
                continue;
            };
            if materialized.contains(&(schedule.id, date)) {
                continue;
            }
            total += i64::from(convert_amount(
                schedule.amount,
                schedule.currency,
                display_currency,
                rates,
            ));
        }
    }
    total
}

/// Builds projection rows for the primary schedule's periods overlapping
/// `[projection_start_date, projection_end_date]`, carrying a running balance seeded with
/// `opening_balance`. When the start falls mid-period, that opening period runs on the opening
/// balance alone (see the comments below).
pub fn build_projection_rows(
    inputs: &ProjectionInputs,
    opening_balance: i64,
    projection_start_date: Option<&str>,
    projection_end_date: Option<&str>,
    today: &str,
) -> Vec<ProjectionRow> {
    let schedule = schedule_from_income(&inputs.primary_schedule);
    let periods = get_projection_periods(
        &schedule,
        Some(today),
        projection_start_date,
        projection_end_date,
    );

    let expense_list = to_expense_with_tags(&inputs.expenses);
    let recurring_list = to_recurring_with_tags(&inputs.recurring);
    let planned_list = to_planned_with_tags(&inputs.planned);
    let budget_list = to_budget_with_tags(&inputs.budgets);
    let display_currency = inputs.display_currency;
    let rates = &inputs.rates;

    let mut running_balance = opening_balance;
    let materialized = build_expense_period_materialized(&expense_list, &budget_list);
    let scheduled_keys = scheduled_income_keys(&inputs.income_all);

    periods
        .into_iter()
        .map(|period| {
            let is_past = period.pay_date.as_str() < today;
            // For the opening period (projection starts mid-period) this is the projection start
            // date, so only expenses dated on or after it count.
            let period_start_date = effective_period_start(&period, projection_start_date);
            // The opening period runs on the accounts' starting balance alone: no salary until the
            // next pay date, and its "free" shows that starting balance.
            let is_opening = period_start_date != period.start_date;

            let income_total = if is_opening {
                0
            } else {
                sum_income_in_period(&inputs.income_all, &period, display_currency, rates)
                    + projected_income_in_period(
                        &inputs.schedules,
                        &period,
                        &scheduled_keys,
                        display_currency,
                        rates,
                        today,
                    )
            };

            let expense_items = get_expense_items_in_period(
                &expense_list,
                &recurring_list,
                &planned_list,
                &period,
                display_currency,
                rates,
                today,
                &budget_list,
                &materialized,
                GetExpenseItemsOptions {
                    include_budget_summaries: false,
                    sort: ExpenseItemSort::DateAsc,
                },
            )
            .into_iter()
            .filter(|item| is_on_or_after_start_date(&item.date, Some(&period_start_date)))
            .collect::<Vec<_>>();

            let expense_total: i64 = expense_items
                .iter()
                .map(|item| i64::from(item.converted_amount))
                .sum();
            running_balance += income_total - expense_total;
            let period_free = if is_opening {
                opening_balance
            } else {
                income_total - expense_total
            };

            ProjectionRow {
                pay_date: period.pay_date,
                start_date: period_start_date,
                end_date: period.end_date,
                income_total,
                expense_total,
                period_free,
                cumulative_free: running_balance,
                expense_items,
                is_past,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IncomeSource, PayFrequency};
    use chrono::Utc;
    use std::collections::HashMap;

    fn empty_rates() -> ExchangeRates {
        ExchangeRates {
            base: "usd".to_string(),
            rates: HashMap::new(),
            fetched_at: "2026-06-01".to_string(),
        }
    }

    fn schedule(anchor: &str, amount: i32) -> IncomePayScheduleRow {
        IncomePayScheduleRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "salary".to_string(),
            anchor_date: NaiveDate::parse_from_str(anchor, "%Y-%m-%d").unwrap(),
            frequency: PayFrequency::Monthly,
            amount,
            currency: CurrencyCode::Usd,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            account_id: None,
        }
    }

    fn scheduled_income(schedule_id: Uuid, date: &str, amount: i32, deleted: bool) -> IncomeRow {
        IncomeRow {
            id: Uuid::new_v4(),
            _user_id: Uuid::new_v4(),
            name: "salary".to_string(),
            amount,
            currency: CurrencyCode::Usd,
            source: IncomeSource::Scheduled,
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            schedule_id: Some(schedule_id),
            created_at: Utc::now(),
            _amount_overridden: false,
            deleted_at: deleted.then(Utc::now),
            account_id: None,
        }
    }

    fn build(schedule: &IncomePayScheduleRow, income: &[IncomeRow]) -> Vec<ProjectionRow> {
        let inputs = ProjectionInputs {
            primary_schedule: schedule.clone(),
            schedules: vec![schedule.clone()],
            income_all: income.to_vec(),
            expenses: Vec::new(),
            recurring: Vec::new(),
            planned: Vec::new(),
            budgets: Vec::new(),
            display_currency: CurrencyCode::Usd,
            rates: empty_rates(),
            initial_free_money: 0,
            projection_start_date: None,
            projection_end_date: None,
        };
        build_projection_rows(&inputs, 0, None, None, "2026-06-01")
    }

    #[test]
    fn future_occurrence_is_projected_when_not_materialized() {
        let sched = schedule("2026-01-15", 100_000);
        let rows = build(&sched, &[]);
        assert_eq!(rows[0].pay_date, "2026-06-15");
        assert_eq!(rows[0].income_total, 100_000);
    }

    #[test]
    fn materialized_occurrence_counts_once() {
        let sched = schedule("2026-01-15", 100_000);
        // Amount overridden after materialization; projection must use the actual row, not re-project.
        let income = vec![scheduled_income(sched.id, "2026-06-15", 120_000, false)];
        let rows = build(&sched, &income);
        assert_eq!(rows[0].pay_date, "2026-06-15");
        assert_eq!(rows[0].income_total, 120_000);
    }

    #[test]
    fn soft_deleted_occurrence_is_neither_counted_nor_reprojected() {
        let sched = schedule("2026-01-15", 100_000);
        let income = vec![scheduled_income(sched.id, "2026-06-15", 100_000, true)];
        let rows = build(&sched, &income);
        assert_eq!(rows[0].pay_date, "2026-06-15");
        assert_eq!(rows[0].income_total, 0);
    }
}
