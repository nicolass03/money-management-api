use std::collections::HashSet;

use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow,
};
use crate::services::budget_status::{
    budget_overlaps_period, get_budget_projection_amount, get_budget_projection_period_date,
    is_budget_projection_projected, is_dated_budget,
};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::materialization::{
    build_planned_materialized_set, build_recurring_materialized_set, is_planned_expense_materialized,
    is_recurring_occurrence_materialized, recurring_due_date,
};
use crate::services::pay_periods::{
    get_pay_dates_in_range, get_projection_periods, is_date_in_period, schedule_from_income,
    schedule_from_recurring, PayPeriod, PROJECTION_MONTHS_FORWARD,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionExpenseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_expense_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_id: Option<Uuid>,
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

struct ExpenseWithTags {
    row: ExpenseRow,
    tags: Vec<String>,
}

struct RecurringWithTags {
    row: RecurringExpenseRow,
    tags: Vec<String>,
}

struct PlannedWithTags {
    row: PlannedExpenseRow,
    tags: Vec<String>,
}

struct BudgetWithTags {
    row: BudgetRow,
    tags: Vec<String>,
    spent: i32,
}

struct BuildProjectionInput<'a> {
    primary_schedule: &'a IncomePayScheduleRow,
    income_entries: &'a [IncomeRow],
    expenses: &'a [ExpenseWithTags],
    recurring_expenses: &'a [RecurringWithTags],
    planned_expenses: &'a [PlannedWithTags],
    budgets: &'a [BudgetWithTags],
    display_currency: CurrencyCode,
    rates: &'a ExchangeRates,
    initial_free_money: i32,
    projection_start_date: Option<&'a str>,
    today: &'a str,
}

struct GetExpenseItemsOptions {
    include_budget_summaries: bool,
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

fn build_dated_budget_ids(budgets: &[BudgetWithTags]) -> HashSet<Uuid> {
    budgets
        .iter()
        .filter(|budget| is_dated_budget(budget.row.start_date, budget.row.end_date))
        .map(|budget| budget.row.id)
        .collect()
}

fn get_expense_items_in_period(
    expense_list: &[ExpenseWithTags],
    recurring_list: &[RecurringWithTags],
    planned_list: &[PlannedWithTags],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    today: &str,
    budgets: &[BudgetWithTags],
    options: GetExpenseItemsOptions,
) -> Vec<ProjectionExpenseItem> {
    let mut items = Vec::new();
    let recurring_materialized = build_recurring_materialized_set(
        &expense_list.iter().map(|e| e.row.clone()).collect::<Vec<_>>(),
    );
    let planned_materialized = build_planned_materialized_set(
        &expense_list.iter().map(|e| e.row.clone()).collect::<Vec<_>>(),
    );
    let dated_budget_ids = build_dated_budget_ids(budgets);

    for expense in expense_list {
        let date = expense.row.date.format("%Y-%m-%d").to_string();
        if !is_date_in_period(&date, period) {
            continue;
        }
        if expense
            .row
            .budget_id
            .is_some_and(|id| dated_budget_ids.contains(&id))
        {
            continue;
        }

        let recurring_source = expense
            .row
            .recurring_id
            .and_then(|id| recurring_list.iter().find(|r| r.row.id == id));

        let due_date = expense
            .row
            .scheduled_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .or_else(|| {
                expense
                    .row
                    .recurring_id
                    .map(|_| recurring_due_date(&expense.row))
            });

        items.push(ProjectionExpenseItem {
            id: Some(expense.row.id),
            recurring_id: expense.row.recurring_id,
            planned_expense_id: expense.row.planned_expense_id,
            budget_id: None,
            budget_total: None,
            budget_spent: None,
            is_budget_summary: None,
            name: expense.row.name.clone(),
            date: date.clone(),
            scheduled_date: due_date
                .filter(|due| *due != date)
                .map(|due| due.clone()),
            amount: expense.row.amount,
            currency: expense.row.currency,
            original_amount: recurring_source
                .filter(|_| !expense.row.amount_overridden)
                .map(|r| r.row.amount),
            original_currency: recurring_source
                .filter(|_| !expense.row.amount_overridden)
                .map(|r| r.row.currency),
            converted_amount: to_display(
                expense.row.amount,
                expense.row.currency,
                display_currency,
                rates,
            ),
            tags: expense.tags.clone(),
            is_subscription: expense.row.is_subscription,
            projected: false,
        });
    }

    for recurring in recurring_list {
        let schedule = schedule_from_recurring(&recurring.row);
        let due_dates = get_pay_dates_in_range(&schedule, &period.start_date, &period.end_date);
        for due_date in due_dates {
            if due_date.as_str() <= today {
                continue;
            }
            if is_recurring_occurrence_materialized(
                &recurring_materialized,
                recurring.row.id,
                &due_date,
            ) {
                continue;
            }
            items.push(ProjectionExpenseItem {
                id: None,
                recurring_id: Some(recurring.row.id),
                planned_expense_id: None,
                budget_id: None,
                budget_total: None,
                budget_spent: None,
                is_budget_summary: None,
                name: recurring.row.name.clone(),
                date: due_date.clone(),
                scheduled_date: None,
                amount: recurring.row.amount,
                currency: recurring.row.currency,
                original_amount: None,
                original_currency: None,
                converted_amount: to_display(
                    recurring.row.amount,
                    recurring.row.currency,
                    display_currency,
                    rates,
                ),
                tags: recurring.tags.clone(),
                is_subscription: recurring.row.is_subscription,
                projected: true,
            });
        }
    }

    for planned in planned_list {
        let date = planned.row.date.format("%Y-%m-%d").to_string();
        if !is_date_in_period(&date, period) || date.as_str() <= today {
            continue;
        }
        if is_planned_expense_materialized(&planned_materialized, planned.row.id) {
            continue;
        }
        items.push(ProjectionExpenseItem {
            id: None,
            recurring_id: None,
            planned_expense_id: Some(planned.row.id),
            budget_id: None,
            budget_total: None,
            budget_spent: None,
            is_budget_summary: None,
            name: planned.row.name.clone(),
            date: date.clone(),
            scheduled_date: None,
            amount: planned.row.amount,
            currency: planned.row.currency,
            original_amount: None,
            original_currency: None,
            converted_amount: to_display(
                planned.row.amount,
                planned.row.currency,
                display_currency,
                rates,
            ),
            tags: planned.tags.clone(),
            is_subscription: false,
            projected: true,
        });
    }

    if !options.include_budget_summaries {
        for budget in budgets {
            if !is_dated_budget(budget.row.start_date, budget.row.end_date) {
                continue;
            }
            let spent = budget.spent;
            let projection_amount = get_budget_projection_amount(
                budget.row.amount,
                budget.row.end_date,
                spent,
                today,
            );
            let end_s = budget.row.end_date.unwrap().format("%Y-%m-%d").to_string();
            if projection_amount <= 0 && today > end_s.as_str() {
                continue;
            }
            let anchor_date = get_budget_projection_period_date(
                budget.row.start_date,
                budget.row.end_date,
                today,
            );
            let Some(anchor_date) = anchor_date else {
                continue;
            };
            if !is_date_in_period(&anchor_date, period) || projection_amount <= 0 {
                continue;
            }
            items.push(ProjectionExpenseItem {
                id: None,
                recurring_id: None,
                planned_expense_id: None,
                budget_id: Some(budget.row.id),
                budget_total: Some(budget.row.amount),
                budget_spent: Some(spent),
                is_budget_summary: Some(false),
                name: budget.row.name.clone(),
                date: anchor_date,
                scheduled_date: None,
                amount: projection_amount,
                currency: budget.row.currency,
                original_amount: None,
                original_currency: None,
                converted_amount: to_display(
                    projection_amount,
                    budget.row.currency,
                    display_currency,
                    rates,
                ),
                tags: budget.tags.clone(),
                is_subscription: false,
                projected: is_budget_projection_projected(
                    budget.row.start_date,
                    budget.row.end_date,
                    today,
                ),
            });
        }
    }

    if options.include_budget_summaries {
        for budget in budgets {
            if !is_dated_budget(budget.row.start_date, budget.row.end_date) {
                continue;
            }
            if !budget_overlaps_period(budget.row.start_date, budget.row.end_date, period) {
                continue;
            }
            let spent = budget.spent;
            items.push(ProjectionExpenseItem {
                id: None,
                recurring_id: None,
                planned_expense_id: None,
                budget_id: Some(budget.row.id),
                budget_total: Some(budget.row.amount),
                budget_spent: Some(spent),
                is_budget_summary: Some(true),
                name: budget.row.name.clone(),
                date: budget.row.start_date.unwrap().format("%Y-%m-%d").to_string(),
                scheduled_date: None,
                amount: spent,
                currency: budget.row.currency,
                original_amount: None,
                original_currency: None,
                converted_amount: to_display(spent, budget.row.currency, display_currency, rates),
                tags: budget.tags.clone(),
                is_subscription: false,
                projected: false,
            });
        }
    }

    items.sort_by(|a, b| a.date.cmp(&b.date));
    items
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
    let expense_list: Vec<ExpenseWithTags> = expenses
        .iter()
        .map(|(row, tags)| ExpenseWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect();
    let recurring_list: Vec<RecurringWithTags> = recurring_expenses
        .iter()
        .map(|(row, tags)| RecurringWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect();
    let planned_list: Vec<PlannedWithTags> = planned_expenses
        .iter()
        .map(|(row, tags)| PlannedWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect();
    let budget_list: Vec<BudgetWithTags> = budgets
        .iter()
        .map(|(row, tags, spent)| BudgetWithTags {
            row: row.clone(),
            tags: tags.clone(),
            spent: *spent,
        })
        .collect();

    let input = BuildProjectionInput {
        primary_schedule,
        income_entries,
        expenses: &expense_list,
        recurring_expenses: &recurring_list,
        planned_expenses: &planned_list,
        budgets: &budget_list,
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

    let mut running_balance = input.initial_free_money;

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
                input.expenses,
                input.recurring_expenses,
                input.planned_expenses,
                &period,
                input.display_currency,
                input.rates,
                input.today,
                input.budgets,
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
