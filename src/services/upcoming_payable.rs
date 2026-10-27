use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::models::{CurrencyCode, ExpenseRow, PlannedExpenseRow, RecurringExpenseRow};
use crate::services::materialization::{
    build_planned_materialized_set, build_recurring_materialized_set,
};
use crate::services::pay_periods::{add_days, get_pay_dates_in_range, schedule_from_recurring};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayableFutureItem {
    pub key: String,
    pub source_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_expense_id: Option<Uuid>,
    pub scheduled_date: NaiveDate,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub tags: Vec<String>,
    pub is_subscription: bool,
}

/// Unpaid recurring/planned charges due in `(today, today + horizon_days]`.
pub fn build_upcoming_payable_items(
    expenses: &[(ExpenseRow, Vec<String>)],
    recurring_expenses: &[(RecurringExpenseRow, Vec<String>)],
    planned_expenses: &[(PlannedExpenseRow, Vec<String>)],
    today: NaiveDate,
    horizon_days: i32,
) -> Vec<PayableFutureItem> {
    let window_end = add_days(today, i64::from(horizon_days));
    let recurring_materialized = build_recurring_materialized_set(expenses.iter().map(|(row, _)| row));
    let planned_materialized = build_planned_materialized_set(expenses.iter().map(|(row, _)| row));
    let mut items = Vec::new();

    for (recurring, tags) in recurring_expenses {
        // `get_pay_dates_in_range` already stops at the template's last payment date.
        let schedule = schedule_from_recurring(recurring);
        for due_date in get_pay_dates_in_range(&schedule, add_days(today, 1), window_end) {
            if recurring_materialized.contains(&(recurring.id, due_date)) {
                continue;
            }

            items.push(PayableFutureItem {
                key: format!("recurring:{}:{due_date}", recurring.id),
                source_type: "recurring".to_string(),
                recurring_id: Some(recurring.id),
                planned_expense_id: None,
                scheduled_date: due_date,
                name: recurring.name.clone(),
                amount: recurring.amount,
                currency: recurring.currency,
                tags: tags.clone(),
                is_subscription: recurring.is_subscription,
            });
        }
    }

    for (planned, tags) in planned_expenses {
        // Undated items have no due date to surface; they're paid from the one-time list.
        let Some(date) = planned.date else { continue };
        if date <= today || date > window_end || planned_materialized.contains(&planned.id) {
            continue;
        }

        items.push(PayableFutureItem {
            key: format!("planned:{}:{date}", planned.id),
            source_type: "planned".to_string(),
            recurring_id: None,
            planned_expense_id: Some(planned.id),
            scheduled_date: date,
            name: planned.name.clone(),
            amount: planned.amount,
            currency: planned.currency,
            tags: tags.clone(),
            is_subscription: false,
        });
    }

    items.sort_by(|a, b| {
        a.scheduled_date
            .cmp(&b.scheduled_date)
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}
