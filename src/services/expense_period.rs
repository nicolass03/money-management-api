use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    BudgetRow, CurrencyCode, ExpenseRow, IncomePayScheduleRow, PlannedExpenseRow,
    RecurringExpenseRow,
};
use crate::services::budget_status::{
    budget_overlaps_period, get_budget_projection_amount, is_budget_projection_projected,
    is_dated_budget,
};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::materialization::{
    build_planned_materialized_set, build_recurring_materialized_set,
};
use crate::services::pay_periods::{
    add_months, get_pay_dates_in_range, get_period_containing, schedule_from_income,
    schedule_from_recurring, PayPeriod,
};
use crate::services::projections::{sort_projection_expense_items, ProjectionExpenseItem};

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
    pub total_spend: i64,
    pub is_pay_period: bool,
    pub by_tag: Vec<TagAmountEntry>,
    pub subscription_split: SubscriptionSplit,
    /// Actual unplanned ("extra") spend in the period: persisted expenses not tied to a
    /// recurring, planned, or budget source, converted to the display currency. Always computed
    /// from raw expense rows (never projected items) so it reflects money actually spent.
    pub extra_spent: i64,
    /// The user's configured extra-spent limit in display-currency minor units, or `None` when
    /// unset. Clients only surface the limit comparison for the pay period (`is_pay_period`).
    pub extra_spent_limit: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayPeriodResponse {
    pub pay_date: NaiveDate,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
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
    pub amount: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionSplit {
    pub subscription: i64,
    pub other: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseChartSummaryResponse {
    pub by_tag: Vec<TagAmountEntry>,
    pub subscription_split: SubscriptionSplit,
}

// Borrowed views over the loaded `(row, tags)` tuples, so building items never clones the dataset.
pub(crate) struct ExpenseWithTags<'a> {
    pub row: &'a ExpenseRow,
    pub tags: &'a [String],
}

pub(crate) struct RecurringWithTags<'a> {
    pub row: &'a RecurringExpenseRow,
    pub tags: &'a [String],
}

pub(crate) struct PlannedWithTags<'a> {
    pub row: &'a PlannedExpenseRow,
    pub tags: &'a [String],
}

pub(crate) struct BudgetWithTags<'a> {
    pub row: &'a BudgetRow,
    pub tags: &'a [String],
    pub spent: i32,
}

pub(crate) struct GetExpenseItemsOptions {
    pub include_budget_summaries: bool,
    pub sort: ExpenseItemSort,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ExpenseItemSort {
    /// Expenses tab: recorded rows by newest date (entry time breaks same-day ties).
    DateDesc,
    /// Projections tab: chronological by charge date.
    DateAsc,
}

pub(crate) struct ExpensePeriodMaterialized {
    pub recurring_materialized: HashSet<(Uuid, NaiveDate)>,
    pub planned_materialized: HashSet<Uuid>,
    pub dated_budget_ids: HashSet<Uuid>,
}

pub fn resolve_period_dates(
    period_key: ExpensePeriodKey,
    primary_schedule: Option<&IncomePayScheduleRow>,
    today: NaiveDate,
) -> Option<PayPeriod> {
    match period_key {
        ExpensePeriodKey::LastPeriod => {
            let schedule = primary_schedule?;
            Some(get_period_containing(&schedule_from_income(schedule), today))
        }
        ExpensePeriodKey::LastMonth => Some(PayPeriod::range(add_months(today, -1), today)),
        ExpensePeriodKey::Last3Months => Some(PayPeriod::range(add_months(today, -3), today)),
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
    today: NaiveDate,
    include_projected: bool,
    extra_spent_limit: Option<i32>,
) -> Option<ExpensePeriodViewResponse> {
    let period = resolve_period_dates(period_key, primary_schedule, today)?;
    let expense_list = to_expense_with_tags(expenses);
    let recurring_list = to_recurring_with_tags(recurring_expenses);
    let planned_list = to_planned_with_tags(planned_expenses);
    let budget_list = to_budget_with_tags(budgets);
    let materialized = build_expense_period_materialized(&expense_list, &budget_list);

    let is_pay_period = period_key == ExpensePeriodKey::LastPeriod;
    let items = if is_pay_period {
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
                sort: ExpenseItemSort::DateDesc,
            },
        )
    } else {
        get_actual_expenses_in_period(
            &expense_list,
            &recurring_list,
            &period,
            display_currency,
            rates,
            ExpenseItemSort::DateDesc,
        )
    };

    let items = if include_projected {
        items
    } else {
        items
            .into_iter()
            .filter(|item| !item.projected)
            .collect()
    };

    let total_spend: i64 = items.iter().map(|item| i64::from(item.converted_amount)).sum();

    // Chart aggregates are computed over the period's actual expenses (the chart range is
    // always the resolved period range), reusing the already-loaded expense list — no extra
    // DB pass. Previously served by the separate /expenses/chart-summary endpoint.
    let chart = build_chart_summary(expenses, &period, display_currency, rates);

    let extra_spent = compute_extra_spent(expenses, &period, display_currency, rates);

    Some(ExpensePeriodViewResponse {
        period: period.into(),
        items,
        total_spend,
        is_pay_period,
        by_tag: chart.by_tag,
        subscription_split: chart.subscription_split,
        extra_spent,
        extra_spent_limit,
    })
}

/// Sums actual unplanned ("extra") spend in the resolved period: persisted expense rows whose
/// `recurring_id`, `planned_expense_id`, and `budget_id` are all `None`, converted to the display
/// currency. This intentionally reads the raw expense rows rather than the period `items` so it is
/// unaffected by `include_projected` and budget-summary aggregation.
fn is_manual_extra_expense(row: &ExpenseRow) -> bool {
    row.recurring_id.is_none()
        && row.planned_expense_id.is_none()
        && row.budget_id.is_none()
}

pub(crate) fn compute_extra_spent(
    expenses: &[(ExpenseRow, Vec<String>)],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> i64 {
    expenses
        .iter()
        .filter(|(row, _)| is_manual_extra_expense(row) && period.contains(row.date))
        .map(|(row, _)| i64::from(convert_amount(row.amount, row.currency, display_currency, rates)))
        .sum()
}

/// Per-tag totals, largest first. A multi-tag expense counts toward each of its tags.
fn sum_by_tag<'a>(
    rows: impl IntoIterator<Item = (&'a [String], i64)>,
) -> Vec<TagAmountEntry> {
    let mut tag_totals: HashMap<&'a str, i64> = HashMap::new();
    for (tags, amount) in rows {
        for tag in tags {
            *tag_totals.entry(tag.as_str()).or_insert(0) += amount;
        }
    }
    let mut by_tag: Vec<TagAmountEntry> = tag_totals
        .into_iter()
        .map(|(tag, amount)| TagAmountEntry { tag: tag.to_string(), amount })
        .collect();
    by_tag.sort_by(|a, b| b.amount.cmp(&a.amount));
    by_tag
}

pub fn compute_extra_spent_by_tag(
    expenses: &[(ExpenseRow, Vec<String>)],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> Vec<TagAmountEntry> {
    sum_by_tag(
        expenses
            .iter()
            .filter(|(row, _)| is_manual_extra_expense(row) && period.contains(row.date))
            .map(|(row, tags)| {
                let converted = convert_amount(row.amount, row.currency, display_currency, rates);
                (tags.as_slice(), i64::from(converted))
            }),
    )
}

pub fn build_chart_summary(
    expenses: &[(ExpenseRow, Vec<String>)],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> ExpenseChartSummaryResponse {
    let mut subscription = 0i64;
    let mut other = 0i64;
    let mut tagged = Vec::new();

    for (row, tags) in expenses {
        if !period.contains(row.date) {
            continue;
        }
        let converted = i64::from(convert_amount(row.amount, row.currency, display_currency, rates));
        if row.is_subscription {
            subscription += converted;
        } else {
            other += converted;
        }
        tagged.push((tags.as_slice(), converted));
    }

    ExpenseChartSummaryResponse {
        by_tag: sum_by_tag(tagged),
        subscription_split: SubscriptionSplit { subscription, other },
    }
}

pub(crate) fn build_expense_period_materialized(
    expense_list: &[ExpenseWithTags],
    budgets: &[BudgetWithTags],
) -> ExpensePeriodMaterialized {
    let rows = || expense_list.iter().map(|expense| expense.row);
    ExpensePeriodMaterialized {
        recurring_materialized: build_recurring_materialized_set(rows()),
        planned_materialized: build_planned_materialized_set(rows()),
        dated_budget_ids: build_dated_budget_ids(budgets),
    }
}

/// A persisted (actual) expense row as a list item, converted to the display currency.
fn actual_expense_item(
    expense: &ExpenseWithTags,
    recurring_list: &[RecurringWithTags],
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> ProjectionExpenseItem {
    let row = expense.row;
    // The template's amount is shown as the "original" only while the charge wasn't overridden.
    let recurring_source = row
        .recurring_id
        .filter(|_| !row.amount_overridden)
        .and_then(|id| recurring_list.iter().find(|r| r.row.id == id));

    ProjectionExpenseItem {
        id: Some(row.id),
        recurring_id: row.recurring_id,
        planned_expense_id: row.planned_expense_id,
        budget_id: row.budget_id,
        account_id: row.account_id,
        budget_total: None,
        budget_spent: None,
        is_budget_summary: None,
        name: row.name.clone(),
        // Early payments keep the occurrence they settled.
        scheduled_date: row.scheduled_date.filter(|due| *due != row.date),
        date: row.date,
        amount: row.amount,
        currency: row.currency,
        original_amount: recurring_source.map(|r| r.row.amount),
        original_currency: recurring_source.map(|r| r.row.currency),
        converted_amount: convert_amount(row.amount, row.currency, display_currency, rates),
        tags: expense.tags.to_vec(),
        is_subscription: row.is_subscription,
        projected: false,
        created_at: Some(row.created_at),
    }
}

pub(crate) fn get_expense_items_in_period(
    expense_list: &[ExpenseWithTags],
    recurring_list: &[RecurringWithTags],
    planned_list: &[PlannedWithTags],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    today: NaiveDate,
    budgets: &[BudgetWithTags],
    materialized: &ExpensePeriodMaterialized,
    options: GetExpenseItemsOptions,
) -> Vec<ProjectionExpenseItem> {
    let mut items = Vec::new();
    let recurring_materialized = &materialized.recurring_materialized;
    let planned_materialized = &materialized.planned_materialized;
    let dated_budget_ids = &materialized.dated_budget_ids;

    for expense in expense_list {
        if !period.contains(expense.row.date) {
            continue;
        }
        // Spend against a dated budget is represented by the budget's own line instead.
        if expense
            .row
            .budget_id
            .is_some_and(|id| dated_budget_ids.contains(&id))
        {
            continue;
        }
        items.push(actual_expense_item(expense, recurring_list, display_currency, rates));
    }

    for recurring in recurring_list {
        let schedule = schedule_from_recurring(recurring.row);
        let due_dates = get_pay_dates_in_range(&schedule, period.start_date, period.end_date);
        for due_date in due_dates {
            // Past occurrences are only ever counted through their materialized expense row.
            // Today's still counts as projected until the daily job charges it (same cutoff as
            // projected income), so it's never invisible between midnight and the job's run.
            if due_date < today || recurring_materialized.contains(&(recurring.row.id, due_date)) {
                continue;
            }
            items.push(ProjectionExpenseItem {
                id: None,
                recurring_id: Some(recurring.row.id),
                planned_expense_id: None,
                budget_id: None,
                account_id: None,
                budget_total: None,
                budget_spent: None,
                is_budget_summary: None,
                name: recurring.row.name.clone(),
                date: due_date,
                scheduled_date: None,
                amount: recurring.row.amount,
                currency: recurring.row.currency,
                original_amount: None,
                original_currency: None,
                converted_amount: convert_amount(
                    recurring.row.amount,
                    recurring.row.currency,
                    display_currency,
                    rates,
                ),
                tags: recurring.tags.to_vec(),
                is_subscription: recurring.row.is_subscription,
                projected: true,
                created_at: None,
            });
        }
    }

    for planned in planned_list {
        // Undated items stay out of periods/projections until paid.
        let Some(due) = planned.row.date else { continue };
        if planned_materialized.contains(&planned.row.id) {
            continue;
        }
        // An unpaid item past its date is still owed: project it as due today (in the current
        // period), keeping its original date as `scheduled_date`, until it is paid.
        let (date, scheduled_date) = if due < today { (today, Some(due)) } else { (due, None) };
        if !period.contains(date) {
            continue;
        }
        items.push(ProjectionExpenseItem {
            id: None,
            recurring_id: None,
            planned_expense_id: Some(planned.row.id),
            budget_id: None,
            account_id: None,
            budget_total: None,
            budget_spent: None,
            is_budget_summary: None,
            name: planned.row.name.clone(),
            date,
            scheduled_date,
            amount: planned.row.amount,
            currency: planned.row.currency,
            original_amount: None,
            original_currency: None,
            converted_amount: convert_amount(
                planned.row.amount,
                planned.row.currency,
                display_currency,
                rates,
            ),
            tags: planned.tags.to_vec(),
            is_subscription: false,
            projected: true,
            created_at: None,
        });
    }

    for budget in budgets {
        let (Some(start), Some(end)) = (budget.row.start_date, budget.row.end_date) else {
            continue;
        };
        if options.include_budget_summaries {
            if !budget_overlaps_period(budget.row.start_date, budget.row.end_date, period) {
                continue;
            }
            items.push(ProjectionExpenseItem {
                id: None,
                recurring_id: None,
                planned_expense_id: None,
                budget_id: Some(budget.row.id),
                account_id: None,
                budget_total: Some(budget.row.amount),
                budget_spent: Some(budget.spent),
                is_budget_summary: Some(true),
                name: budget.row.name.clone(),
                date: start,
                scheduled_date: None,
                amount: budget.spent,
                currency: budget.row.currency,
                original_amount: None,
                original_currency: None,
                converted_amount: convert_amount(
                    budget.spent,
                    budget.row.currency,
                    display_currency,
                    rates,
                ),
                tags: budget.tags.to_vec(),
                is_subscription: false,
                projected: false,
                created_at: None,
            });
            continue;
        }

        // Projection line: always on the budget's end date — the full envelope while it runs,
        // actual spent once it has ended or been completed. A fixed date keeps the line in one
        // period for the budget's whole life, and by the time that period closes (and is frozen)
        // the budget has ended, so frozen history never holds a stale envelope amount.
        let projection_amount = get_budget_projection_amount(
            budget.row.amount,
            budget.row.end_date,
            budget.spent,
            today,
            budget.row.completed_at.as_ref(),
        );
        let date = end;
        if projection_amount <= 0 || !period.contains(date) {
            continue;
        }
        items.push(ProjectionExpenseItem {
            id: None,
            recurring_id: None,
            planned_expense_id: None,
            budget_id: Some(budget.row.id),
            account_id: None,
            budget_total: Some(budget.row.amount),
            budget_spent: Some(budget.spent),
            is_budget_summary: Some(false),
            name: budget.row.name.clone(),
            date,
            scheduled_date: None,
            amount: projection_amount,
            currency: budget.row.currency,
            original_amount: None,
            original_currency: None,
            converted_amount: convert_amount(
                projection_amount,
                budget.row.currency,
                display_currency,
                rates,
            ),
            tags: budget.tags.to_vec(),
            is_subscription: false,
            projected: is_budget_projection_projected(
                budget.row.start_date,
                budget.row.end_date,
                today,
            ),
            created_at: None,
        });
    }

    sort_projection_expense_items(&mut items, options.sort);
    items
}

fn get_actual_expenses_in_period(
    expense_list: &[ExpenseWithTags],
    recurring_list: &[RecurringWithTags],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    sort: ExpenseItemSort,
) -> Vec<ProjectionExpenseItem> {
    let mut items: Vec<ProjectionExpenseItem> = expense_list
        .iter()
        .filter(|expense| period.contains(expense.row.date))
        .map(|expense| actual_expense_item(expense, recurring_list, display_currency, rates))
        .collect();

    sort_projection_expense_items(&mut items, sort);
    items
}

fn build_dated_budget_ids(budgets: &[BudgetWithTags]) -> HashSet<Uuid> {
    budgets
        .iter()
        .filter(|budget| is_dated_budget(budget.row.start_date, budget.row.end_date))
        .map(|budget| budget.row.id)
        .collect()
}

pub(crate) fn to_expense_with_tags(expenses: &[(ExpenseRow, Vec<String>)]) -> Vec<ExpenseWithTags<'_>> {
    expenses
        .iter()
        .map(|(row, tags)| ExpenseWithTags { row, tags })
        .collect()
}

pub(crate) fn to_recurring_with_tags(
    recurring: &[(RecurringExpenseRow, Vec<String>)],
) -> Vec<RecurringWithTags<'_>> {
    recurring
        .iter()
        .map(|(row, tags)| RecurringWithTags { row, tags })
        .collect()
}

pub(crate) fn to_planned_with_tags(
    planned: &[(PlannedExpenseRow, Vec<String>)],
) -> Vec<PlannedWithTags<'_>> {
    planned
        .iter()
        .map(|(row, tags)| PlannedWithTags { row, tags })
        .collect()
}

pub(crate) fn to_budget_with_tags(
    budgets: &[(BudgetRow, Vec<String>, i32)],
) -> Vec<BudgetWithTags<'_>> {
    budgets
        .iter()
        .map(|(row, tags, spent)| BudgetWithTags { row, tags, spent: *spent })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn empty_rates() -> ExchangeRates {
        ExchangeRates {
            base: "usd".to_string(),
            rates: HashMap::new(),
            fetched_at: "2026-06-14".to_string(),
        }
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn period() -> PayPeriod {
        PayPeriod::range(date("2026-06-01"), date("2026-06-30"))
    }

    fn expense(amount: i32, on: &str) -> (ExpenseRow, Vec<String>) {
        (
            ExpenseRow {
                id: Uuid::new_v4(),
                _user_id: Uuid::new_v4(),
                name: "test".to_string(),
                amount,
                currency: CurrencyCode::Usd,
                date: date(on),
                scheduled_date: None,
                recurring_id: None,
                planned_expense_id: None,
                budget_id: None,
                amount_overridden: false,
                is_subscription: false,
                created_at: Utc::now(),
                account_id: None,
            },
            Vec::new(),
        )
    }

    #[test]
    fn extra_spent_sums_only_manual_in_period() {
        let mut manual = expense(1000, "2026-06-10");
        let mut recurring = expense(2000, "2026-06-11");
        recurring.0.recurring_id = Some(Uuid::new_v4());
        let mut planned = expense(3000, "2026-06-12");
        planned.0.planned_expense_id = Some(Uuid::new_v4());
        let mut budgeted = expense(4000, "2026-06-13");
        budgeted.0.budget_id = Some(Uuid::new_v4());
        let another_manual = expense(500, "2026-06-14");

        let expenses = vec![manual, recurring, planned, budgeted, another_manual];

        let total = compute_extra_spent(&expenses, &period(), CurrencyCode::Usd, &empty_rates());
        assert_eq!(total, 1500);
    }

    #[test]
    fn extra_spent_excludes_out_of_period() {
        let inside = expense(1000, "2026-06-10");
        let before = expense(9999, "2026-05-31");
        let after = expense(8888, "2026-07-01");

        let expenses = vec![inside, before, after];

        let total = compute_extra_spent(&expenses, &period(), CurrencyCode::Usd, &empty_rates());
        assert_eq!(total, 1000);
    }

    #[test]
    fn extra_spent_is_zero_without_manual() {
        let mut recurring = expense(2000, "2026-06-11");
        recurring.0.recurring_id = Some(Uuid::new_v4());

        let expenses = vec![recurring];

        let total = compute_extra_spent(&expenses, &period(), CurrencyCode::Usd, &empty_rates());
        assert_eq!(total, 0);
    }

    #[test]
    fn last_period_excludes_prior_period_expenses_after_payday() {
        use crate::models::{IncomePayScheduleRow, PayFrequency};

        let schedule = IncomePayScheduleRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "Pay".to_string(),
            anchor_date: NaiveDate::from_ymd_opt(2026, 1, 25).unwrap(),
            frequency: PayFrequency::Monthly,
            amount: 500_000,
            currency: CurrencyCode::Usd,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            account_id: None,
        };

        let prior_period_expense = expense(1200, "2026-06-20");

        let view = build_expense_period_view(
            ExpensePeriodKey::LastPeriod,
            Some(&schedule),
            &[prior_period_expense],
            &[],
            &[],
            &[],
            CurrencyCode::Usd,
            &empty_rates(),
            date("2026-06-26"),
            false,
            None,
        )
        .expect("pay period view");

        assert_eq!(view.period.start_date, date("2026-06-26"));
        assert_eq!(view.period.pay_date, date("2026-07-25"));
        assert!(view.items.is_empty());
    }
}
