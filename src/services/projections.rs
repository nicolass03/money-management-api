use std::collections::HashSet;

use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow,
};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::expense_period::{
    build_expense_period_materialized, get_expense_items_in_period, to_budget_with_tags,
    to_expense_with_tags, to_planned_with_tags, to_recurring_with_tags, GetExpenseItemsOptions,
};
use crate::services::pay_periods::{
    get_pay_dates_in_range, get_projection_periods, is_date_in_period, schedule_from_income,
    PayPeriod, PROJECTION_MONTHS_FORWARD,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionRow {
    pub pay_date: String,
    pub start_date: String,
    pub end_date: String,
    pub income_total: i32,
    pub expense_total: i32,
    pub period_free: i32,
    pub cumulative_free: i32,
    pub expense_items: Vec<ProjectionExpenseItem>,
    pub is_past: bool,
}

struct BuildProjectionInput<'a> {
    primary_schedule: &'a IncomePayScheduleRow,
    schedules: &'a [IncomePayScheduleRow],
    income_entries: &'a [IncomeRow],
    expenses: &'a [(ExpenseRow, Vec<String>)],
    recurring_expenses: &'a [(RecurringExpenseRow, Vec<String>)],
    planned_expenses: &'a [(PlannedExpenseRow, Vec<String>)],
    budgets: &'a [(BudgetRow, Vec<String>, i32)],
    display_currency: CurrencyCode,
    rates: &'a ExchangeRates,
    initial_free_money: i32,
    projection_start_date: Option<&'a str>,
    today: &'a str,
}

fn to_display(
    amount: i32,
    currency: CurrencyCode,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> i32 {
    convert_amount(amount, currency, display_currency, rates)
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

fn is_opening_partial_period(period: &PayPeriod, projection_start_date: Option<&str>) -> bool {
    projection_start_date.is_some_and(|start| {
        start > period.start_date.as_str() && start <= period.end_date.as_str()
    })
}

fn sum_income_in_period(
    entries: &[IncomeRow],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    min_date: Option<&str>,
    max_date: Option<&str>,
) -> i32 {
    entries
        .iter()
        .filter(|entry| entry.deleted_at.is_none())
        .filter(|entry| {
            let date = entry.date.format("%Y-%m-%d").to_string();
            is_date_in_period(&date, period)
                && is_on_or_after_start_date(&date, min_date)
                && max_date.is_none_or(|max| date.as_str() <= max)
        })
        .map(|entry| to_display(entry.amount, entry.currency, display_currency, rates))
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
) -> i32 {
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
            total += to_display(schedule.amount, schedule.currency, display_currency, rates);
        }
    }
    total
}

#[allow(clippy::too_many_arguments)]
pub fn build_projection_rows(
    primary_schedule: &IncomePayScheduleRow,
    schedules: &[IncomePayScheduleRow],
    income_entries: &[IncomeRow],
    expenses: &[(ExpenseRow, Vec<String>)],
    recurring_expenses: &[(RecurringExpenseRow, Vec<String>)],
    planned_expenses: &[(PlannedExpenseRow, Vec<String>)],
    budgets: &[(BudgetRow, Vec<String>, i32)],
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    initial_free_money: i32,
    projection_start_date: Option<&str>,
    today: &str,
) -> Vec<ProjectionRow> {
    let input = BuildProjectionInput {
        primary_schedule,
        schedules,
        income_entries,
        expenses,
        recurring_expenses,
        planned_expenses,
        budgets,
        display_currency,
        rates,
        initial_free_money,
        projection_start_date,
        today,
    };

    build_projection_rows_inner(input)
}

fn build_projection_rows_inner(input: BuildProjectionInput<'_>) -> Vec<ProjectionRow> {
    let schedule = schedule_from_income(input.primary_schedule);
    let periods = get_projection_periods(
        &schedule,
        Some(input.today),
        input.projection_start_date,
        PROJECTION_MONTHS_FORWARD,
    );

    let expense_list = to_expense_with_tags(input.expenses);
    let recurring_list = to_recurring_with_tags(input.recurring_expenses);
    let planned_list = to_planned_with_tags(input.planned_expenses);
    let budget_list = to_budget_with_tags(input.budgets);

    let mut running_balance = input.initial_free_money;
    let materialized = build_expense_period_materialized(&expense_list, &budget_list);
    let scheduled_keys = scheduled_income_keys(input.income_entries);

    periods
        .into_iter()
        .map(|period| {
            let is_past = period.pay_date.as_str() < input.today;
            let period_start_date = effective_period_start(&period, input.projection_start_date);
            let opening_partial = is_opening_partial_period(&period, input.projection_start_date);
            let min_activity_date = if opening_partial {
                input.projection_start_date.unwrap().to_string()
            } else {
                period_start_date.clone()
            };

            let income_total = if opening_partial {
                0
            } else {
                let actual = sum_income_in_period(
                    input.income_entries,
                    &period,
                    input.display_currency,
                    input.rates,
                    Some(&period.start_date),
                    None,
                );
                let projected = projected_income_in_period(
                    input.schedules,
                    &period,
                    &scheduled_keys,
                    input.display_currency,
                    input.rates,
                    input.today,
                );
                actual + projected
            };

            let expense_items = get_expense_items_in_period(
                &expense_list,
                &recurring_list,
                &planned_list,
                &period,
                input.display_currency,
                input.rates,
                input.today,
                &budget_list,
                &materialized,
                GetExpenseItemsOptions {
                    include_budget_summaries: false,
                },
            )
            .into_iter()
            .filter(|item| is_on_or_after_start_date(&item.date, Some(&min_activity_date)))
            .collect::<Vec<_>>();

            let expense_total: i32 = expense_items.iter().map(|item| item.converted_amount).sum();
            let period_free = income_total - expense_total;
            running_balance += period_free;
            let cumulative_free = if opening_partial {
                input.initial_free_money
            } else {
                running_balance
            };

            ProjectionRow {
                pay_date: period.pay_date,
                start_date: period_start_date,
                end_date: period.end_date,
                income_total,
                expense_total,
                period_free,
                cumulative_free,
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
        }
    }

    fn build(schedule: &IncomePayScheduleRow, income: &[IncomeRow]) -> Vec<ProjectionRow> {
        build_projection_rows(
            schedule,
            std::slice::from_ref(schedule),
            income,
            &[],
            &[],
            &[],
            &[],
            CurrencyCode::Usd,
            &empty_rates(),
            0,
            None,
            "2026-06-01",
        )
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
