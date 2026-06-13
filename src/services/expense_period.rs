use std::collections::HashSet;

use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, PlannedExpenseRow,
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
    add_months, get_pay_dates_in_range, get_period_containing, is_date_in_period,
    schedule_from_income, schedule_from_recurring, PayPeriod,
};
use crate::services::projections::ProjectionExpenseItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpensePeriodKey {
    LastPeriod,
    LastMonth,
    Last3Months,
}

impl ExpensePeriodKey {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "last-period" => Some(Self::LastPeriod),
            "last-month" => Some(Self::LastMonth),
            "last-3-months" => Some(Self::Last3Months),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpensePeriodViewResponse {
    pub period: PayPeriodResponse,
    pub items: Vec<ProjectionExpenseItem>,
    pub total_spend: i32,
    pub is_pay_period: bool,
    pub by_tag: Vec<TagAmountEntry>,
    pub subscription_split: SubscriptionSplit,
    #[serde(skip_serializing_if = "is_zero")]
    pub extra_spend: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_spend_limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_spend_limit_currency: Option<CurrencyCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_spend_limit_converted: Option<i32>,
    #[serde(skip_serializing_if = "is_zero")]
    pub planned_spend: i32,
    #[serde(skip_serializing_if = "is_zero")]
    pub planned_total: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_used_percent: Option<i32>,
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayPeriodResponse {
    pub pay_date: String,
    pub start_date: String,
    pub end_date: String,
}

impl From<PayPeriod> for PayPeriodResponse {
    fn from(period: PayPeriod) -> Self {
        Self {
            pay_date: period.pay_date,
            start_date: period.start_date,
            end_date: period.end_date,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagAmountEntry {
    pub tag: String,
    pub amount: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionSplit {
    pub subscription: i32,
    pub other: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseChartSummaryResponse {
    pub by_tag: Vec<TagAmountEntry>,
    pub subscription_split: SubscriptionSplit,
}

pub(crate) struct ExpenseWithTags {
    pub row: ExpenseRow,
    pub tags: Vec<String>,
}

pub(crate) struct RecurringWithTags {
    pub row: RecurringExpenseRow,
    pub tags: Vec<String>,
}

pub(crate) struct PlannedWithTags {
    pub row: PlannedExpenseRow,
    pub tags: Vec<String>,
}

pub(crate) struct BudgetWithTags {
    pub row: BudgetRow,
    pub tags: Vec<String>,
    pub spent: i32,
}

pub(crate) struct GetExpenseItemsOptions {
    pub include_budget_summaries: bool,
}

pub(crate) struct ExpensePeriodMaterialized {
    pub recurring_materialized: HashSet<String>,
    pub planned_materialized: HashSet<Uuid>,
    pub dated_budget_ids: HashSet<Uuid>,
}

#[derive(Debug, Clone, Default)]
struct PayPeriodStats {
    extra_spend: i32,
    extra_spend_limit: Option<i32>,
    extra_spend_limit_currency: Option<CurrencyCode>,
    extra_spend_limit_converted: Option<i32>,
    planned_spend: i32,
    planned_total: i32,
    planned_used_percent: Option<i32>,
}

fn is_extra_expense_row(row: &ExpenseRow) -> bool {
    row.recurring_id.is_none() && row.planned_expense_id.is_none() && row.budget_id.is_none()
}

fn is_planned_item(item: &ProjectionExpenseItem) -> bool {
    item.recurring_id.is_some() || item.planned_expense_id.is_some() || item.budget_id.is_some()
}

pub(crate) fn compute_pay_period_stats(
    expense_list: &[ExpenseWithTags],
    all_items: &[ProjectionExpenseItem],
    period: &PayPeriod,
    extra_limit: Option<i32>,
    extra_limit_currency: Option<CurrencyCode>,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> PayPeriodStats {
    let extra_spend: i32 = expense_list
        .iter()
        .filter(|expense| {
            let date = expense.row.date.format("%Y-%m-%d").to_string();
            is_date_in_period(&date, period) && is_extra_expense_row(&expense.row)
        })
        .map(|expense| {
            to_display(
                expense.row.amount,
                expense.row.currency,
                display_currency,
                rates,
            )
        })
        .sum();

    let mut planned_total = 0i32;
    let mut planned_spend = 0i32;

    for item in all_items {
        if !is_planned_item(item) {
            continue;
        }
        if item.is_budget_summary == Some(true) {
            if let (Some(total), Some(spent)) = (item.budget_total, item.budget_spent) {
                planned_total += to_display(total, item.currency, display_currency, rates);
                planned_spend += to_display(spent, item.currency, display_currency, rates);
            }
        } else {
            planned_total += item.converted_amount;
            if !item.projected {
                planned_spend += item.converted_amount;
            }
        }
    }

    let planned_used_percent = if planned_total > 0 {
        Some(((planned_spend as f64 / planned_total as f64) * 100.0).round() as i32)
    } else {
        None
    };

    let extra_spend_limit_converted = match (extra_limit, extra_limit_currency) {
        (Some(amount), Some(currency)) => Some(to_display(amount, currency, display_currency, rates)),
        _ => None,
    };

    PayPeriodStats {
        extra_spend,
        extra_spend_limit: extra_limit,
        extra_spend_limit_currency: extra_limit_currency,
        extra_spend_limit_converted,
        planned_spend,
        planned_total,
        planned_used_percent,
    }
}

pub fn resolve_period_dates(
    period_key: ExpensePeriodKey,
    primary_schedule: Option<&IncomePayScheduleRow>,
    today: &str,
) -> Option<PayPeriod> {
    match period_key {
        ExpensePeriodKey::LastPeriod => {
            let schedule = primary_schedule?;
            let input = schedule_from_income(schedule);
            Some(get_period_containing(&input, today))
        }
        ExpensePeriodKey::LastMonth => Some(PayPeriod {
            pay_date: today.to_string(),
            start_date: add_months(today, -1),
            end_date: today.to_string(),
        }),
        ExpensePeriodKey::Last3Months => Some(PayPeriod {
            pay_date: today.to_string(),
            start_date: add_months(today, -3),
            end_date: today.to_string(),
        }),
    }
}

pub fn build_expense_period_view(
    period_key: ExpensePeriodKey,
    primary_schedule: Option<&IncomePayScheduleRow>,
    expenses: &[(ExpenseRow, Vec<String>)],
    recurring_expenses: &[(RecurringExpenseRow, Vec<String>)],
    planned_expenses: &[(PlannedExpenseRow, Vec<String>)],
    budgets: &[(BudgetRow, Vec<String>, i32)],
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    today: &str,
    include_projected: bool,
    extra_expense_limit: Option<i32>,
    extra_expense_limit_currency: Option<CurrencyCode>,
) -> Option<ExpensePeriodViewResponse> {
    let period = resolve_period_dates(period_key, primary_schedule, today)?;
    let expense_list = to_expense_with_tags(expenses);
    let recurring_list = to_recurring_with_tags(recurring_expenses);
    let planned_list = to_planned_with_tags(planned_expenses);
    let budget_list = to_budget_with_tags(budgets);
    let materialized = build_expense_period_materialized(&expense_list, &budget_list);

    let is_pay_period = period_key == ExpensePeriodKey::LastPeriod;
    let all_items = if is_pay_period {
        get_expense_items_in_period(
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
                include_budget_summaries: true,
            },
        )
    } else {
        get_actual_expenses_in_date_range(
            &expense_list,
            &recurring_list,
            &period.start_date,
            &period.end_date,
            display_currency,
            rates,
        )
    };

    let pay_period_stats = if is_pay_period {
        compute_pay_period_stats(
            &expense_list,
            &all_items,
            &period,
            extra_expense_limit,
            extra_expense_limit_currency,
            display_currency,
            rates,
        )
    } else {
        PayPeriodStats::default()
    };

    let items = if include_projected {
        all_items
    } else {
        all_items
            .into_iter()
            .filter(|item| !item.projected)
            .collect()
    };

    let total_spend: i32 = items.iter().map(|item| item.converted_amount).sum();

    // Chart aggregates are computed over the period's actual expenses (the chart range is
    // always the resolved period range), reusing the already-loaded expense list — no extra
    // DB pass. Previously served by the separate /expenses/chart-summary endpoint.
    let chart = build_chart_summary(
        expenses,
        &period.start_date,
        &period.end_date,
        display_currency,
        rates,
    );

    Some(ExpensePeriodViewResponse {
        period: period.into(),
        items,
        total_spend,
        is_pay_period,
        by_tag: chart.by_tag,
        subscription_split: chart.subscription_split,
        extra_spend: pay_period_stats.extra_spend,
        extra_spend_limit: pay_period_stats.extra_spend_limit,
        extra_spend_limit_currency: pay_period_stats.extra_spend_limit_currency,
        extra_spend_limit_converted: pay_period_stats.extra_spend_limit_converted,
        planned_spend: pay_period_stats.planned_spend,
        planned_total: pay_period_stats.planned_total,
        planned_used_percent: pay_period_stats.planned_used_percent,
    })
}

pub fn build_chart_summary(
    expenses: &[(ExpenseRow, Vec<String>)],
    from: &str,
    to: &str,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> ExpenseChartSummaryResponse {
    let period = PayPeriod {
        pay_date: to.to_string(),
        start_date: from.to_string(),
        end_date: to.to_string(),
    };

    let mut tag_totals: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut subscription = 0i32;
    let mut other = 0i32;

    for (row, tags) in expenses {
        let date = row.date.format("%Y-%m-%d").to_string();
        if !is_date_in_period(&date, &period) {
            continue;
        }
        let converted = convert_amount(row.amount, row.currency, display_currency, rates);
        if row.is_subscription {
            subscription += converted;
        } else {
            other += converted;
        }
        for tag in tags {
            *tag_totals.entry(tag.clone()).or_insert(0) += converted;
        }
    }

    let mut by_tag: Vec<TagAmountEntry> = tag_totals
        .into_iter()
        .map(|(tag, amount)| TagAmountEntry { tag, amount })
        .collect();
    by_tag.sort_by(|a, b| b.amount.cmp(&a.amount));

    ExpenseChartSummaryResponse {
        by_tag,
        subscription_split: SubscriptionSplit { subscription, other },
    }
}

pub(crate) fn build_expense_period_materialized(
    expense_list: &[ExpenseWithTags],
    budgets: &[BudgetWithTags],
) -> ExpensePeriodMaterialized {
    let expense_rows: Vec<ExpenseRow> = expense_list.iter().map(|e| e.row.clone()).collect();
    ExpensePeriodMaterialized {
        recurring_materialized: build_recurring_materialized_set(&expense_rows),
        planned_materialized: build_planned_materialized_set(&expense_rows),
        dated_budget_ids: build_dated_budget_ids(budgets),
    }
}

pub(crate) fn get_expense_items_in_period(
    expense_list: &[ExpenseWithTags],
    recurring_list: &[RecurringWithTags],
    planned_list: &[PlannedWithTags],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    today: &str,
    budgets: &[BudgetWithTags],
    materialized: &ExpensePeriodMaterialized,
    options: GetExpenseItemsOptions,
) -> Vec<ProjectionExpenseItem> {
    let mut items = Vec::new();
    let recurring_materialized = &materialized.recurring_materialized;
    let planned_materialized = &materialized.planned_materialized;
    let dated_budget_ids = &materialized.dated_budget_ids;

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
            scheduled_date: due_date.filter(|due| *due != date),
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
                recurring_materialized,
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
        if is_planned_expense_materialized(planned_materialized, planned.row.id) {
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

fn get_actual_expenses_in_date_range(
    expense_list: &[ExpenseWithTags],
    recurring_list: &[RecurringWithTags],
    start_date: &str,
    end_date: &str,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> Vec<ProjectionExpenseItem> {
    let period = PayPeriod {
        pay_date: end_date.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
    };

    expense_list
        .iter()
        .filter(|expense| {
            let date = expense.row.date.format("%Y-%m-%d").to_string();
            is_date_in_period(&date, &period)
        })
        .map(|expense| {
            let date = expense.row.date.format("%Y-%m-%d").to_string();
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

            ProjectionExpenseItem {
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
            }
        })
        .collect()
}

fn to_display(
    amount: i32,
    currency: CurrencyCode,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> i32 {
    convert_amount(amount, currency, display_currency, rates)
}

fn build_dated_budget_ids(budgets: &[BudgetWithTags]) -> HashSet<Uuid> {
    budgets
        .iter()
        .filter(|budget| is_dated_budget(budget.row.start_date, budget.row.end_date))
        .map(|budget| budget.row.id)
        .collect()
}

pub(crate) fn to_expense_with_tags(expenses: &[(ExpenseRow, Vec<String>)]) -> Vec<ExpenseWithTags> {
    expenses
        .iter()
        .map(|(row, tags)| ExpenseWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect()
}

pub(crate) fn to_recurring_with_tags(
    recurring: &[(RecurringExpenseRow, Vec<String>)],
) -> Vec<RecurringWithTags> {
    recurring
        .iter()
        .map(|(row, tags)| RecurringWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect()
}

pub(crate) fn to_planned_with_tags(
    planned: &[(PlannedExpenseRow, Vec<String>)],
) -> Vec<PlannedWithTags> {
    planned
        .iter()
        .map(|(row, tags)| PlannedWithTags {
            row: row.clone(),
            tags: tags.clone(),
        })
        .collect()
}

pub(crate) fn to_budget_with_tags(budgets: &[(BudgetRow, Vec<String>, i32)]) -> Vec<BudgetWithTags> {
    budgets
        .iter()
        .map(|(row, tags, spent)| BudgetWithTags {
            row: row.clone(),
            tags: tags.clone(),
            spent: *spent,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{NaiveDate, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::services::currency::ExchangeRates;
    use crate::services::projections::ProjectionExpenseItem;

    fn identity_rates() -> ExchangeRates {
        ExchangeRates {
            base: "USD".into(),
            rates: HashMap::from([
                ("USD".to_string(), 1.0),
                ("EUR".to_string(), 1.0),
                ("COP".to_string(), 1.0),
            ]),
            fetched_at: "2025-01-01".into(),
        }
    }

    fn pay_period() -> PayPeriod {
        PayPeriod {
            pay_date: "2025-06-15".into(),
            start_date: "2025-06-01".into(),
            end_date: "2025-06-30".into(),
        }
    }

    fn expense_row(
        amount: i32,
        date: &str,
        recurring_id: Option<Uuid>,
        planned_expense_id: Option<Uuid>,
        budget_id: Option<Uuid>,
    ) -> ExpenseRow {
        ExpenseRow {
            id: Uuid::new_v4(),
            _user_id: Uuid::new_v4(),
            name: "test".into(),
            amount,
            currency: CurrencyCode::Usd,
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            scheduled_date: None,
            recurring_id,
            planned_expense_id,
            budget_id,
            amount_overridden: false,
            is_subscription: false,
            created_at: Utc::now(),
        }
    }

    fn projection_item(
        converted_amount: i32,
        projected: bool,
        recurring_id: Option<Uuid>,
        budget_total: Option<i32>,
        budget_spent: Option<i32>,
        is_budget_summary: bool,
    ) -> ProjectionExpenseItem {
        ProjectionExpenseItem {
            id: None,
            recurring_id,
            planned_expense_id: None,
            budget_id: if is_budget_summary {
                Some(Uuid::new_v4())
            } else {
                recurring_id
            },
            budget_total,
            budget_spent,
            is_budget_summary: is_budget_summary.then_some(true),
            name: "item".into(),
            date: "2025-06-10".into(),
            scheduled_date: None,
            amount: converted_amount,
            currency: CurrencyCode::Usd,
            original_amount: None,
            original_currency: None,
            converted_amount,
            tags: vec![],
            is_subscription: false,
            projected,
        }
    }

    #[test]
    fn extra_spend_counts_only_manual_expenses() {
        let period = pay_period();
        let expenses = vec![
            ExpenseWithTags {
                row: expense_row(1000, "2025-06-10", None, None, None),
                tags: vec![],
            },
            ExpenseWithTags {
                row: expense_row(500, "2025-06-12", Some(Uuid::new_v4()), None, None),
                tags: vec![],
            },
            ExpenseWithTags {
                row: expense_row(200, "2025-07-01", None, None, None),
                tags: vec![],
            },
        ];

        let stats = compute_pay_period_stats(
            &expenses,
            &[],
            &period,
            None,
            None,
            CurrencyCode::Usd,
            &identity_rates(),
        );

        assert_eq!(stats.extra_spend, 1000);
    }

    #[test]
    fn planned_used_percent_includes_projected_in_total() {
        let stats = compute_pay_period_stats(
            &[],
            &[
                projection_item(3000, false, Some(Uuid::new_v4()), None, None, false),
                projection_item(2000, true, Some(Uuid::new_v4()), None, None, false),
            ],
            &pay_period(),
            None,
            None,
            CurrencyCode::Usd,
            &identity_rates(),
        );

        assert_eq!(stats.planned_spend, 3000);
        assert_eq!(stats.planned_total, 5000);
        assert_eq!(stats.planned_used_percent, Some(60));
    }

    #[test]
    fn budget_summary_uses_budget_totals() {
        let stats = compute_pay_period_stats(
            &[],
            &[projection_item(
                4000,
                false,
                None,
                Some(10_000),
                Some(4000),
                true,
            )],
            &pay_period(),
            None,
            None,
            CurrencyCode::Usd,
            &identity_rates(),
        );

        assert_eq!(stats.planned_spend, 4000);
        assert_eq!(stats.planned_total, 10_000);
        assert_eq!(stats.planned_used_percent, Some(40));
    }

    #[test]
    fn zero_planned_total_yields_none_percent() {
        let stats = compute_pay_period_stats(
            &[],
            &[],
            &pay_period(),
            None,
            None,
            CurrencyCode::Usd,
            &identity_rates(),
        );

        assert!(stats.planned_used_percent.is_none());
    }

    #[test]
    fn extra_limit_converts_to_display_currency() {
        let stats = compute_pay_period_stats(
            &[],
            &[],
            &pay_period(),
            Some(5000),
            Some(CurrencyCode::Usd),
            CurrencyCode::Usd,
            &identity_rates(),
        );

        assert_eq!(stats.extra_spend_limit_converted, Some(5000));
    }
}
