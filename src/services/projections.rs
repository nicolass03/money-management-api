use serde::Serialize;

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
    get_projection_periods, is_date_in_period, schedule_from_income, PayPeriod,
    PROJECTION_MONTHS_FORWARD,
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
        .filter(|entry| {
            let date = entry.date.format("%Y-%m-%d").to_string();
            is_date_in_period(&date, period)
                && is_on_or_after_start_date(&date, min_date)
                && max_date.is_none_or(|max| date.as_str() <= max)
        })
        .map(|entry| to_display(entry.amount, entry.currency, display_currency, rates))
        .sum()
}

pub fn build_projection_rows(
    primary_schedule: &IncomePayScheduleRow,
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
                sum_income_in_period(
                    input.income_entries,
                    &period,
                    input.display_currency,
                    input.rates,
                    Some(&period.start_date),
                    None,
                )
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
