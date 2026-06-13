use serde::Serialize;
use uuid::Uuid;

use crate::models::{CurrencyCode, ExpenseRow, PlannedExpenseRow, RecurringExpenseRow};
use crate::services::materialization::{
    build_planned_materialized_set, build_recurring_materialized_set, is_planned_expense_materialized,
    is_recurring_occurrence_materialized,
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
    pub scheduled_date: String,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub tags: Vec<String>,
    pub is_subscription: bool,
}

pub fn build_upcoming_payable_items(
    expenses: &[(ExpenseRow, Vec<String>)],
    recurring_expenses: &[(RecurringExpenseRow, Vec<String>)],
    planned_expenses: &[(PlannedExpenseRow, Vec<String>)],
    today: &str,
    horizon_days: i32,
) -> Vec<PayableFutureItem> {
    let window_start = today;
    let window_end = add_days(today, horizon_days);
    let expense_rows: Vec<ExpenseRow> = expenses.iter().map(|(row, _)| row.clone()).collect();
    let recurring_materialized = build_recurring_materialized_set(&expense_rows);
    let planned_materialized = build_planned_materialized_set(&expense_rows);
    let mut items = Vec::new();

    for (recurring, tags) in recurring_expenses {
        if let Some(last) = recurring.last_payment_date {
            let last_s = last.format("%Y-%m-%d").to_string();
            if window_start > last_s.as_str() {
                continue;
            }
        }

        let schedule = schedule_from_recurring(recurring);
        let due_dates = get_pay_dates_in_range(
            &schedule,
            &add_days(window_start, 1),
            &window_end,
        );

        for due_date in due_dates {
            if due_date.as_str() <= today {
                continue;
            }

            if let Some(last) = recurring.last_payment_date {
                let last_s = last.format("%Y-%m-%d").to_string();
                if due_date.as_str() > last_s.as_str() {
                    continue;
                }
            }

            if is_recurring_occurrence_materialized(
                &recurring_materialized,
                recurring.id,
                &due_date,
            ) {
                continue;
            }

            items.push(PayableFutureItem {
                key: format!("recurring:{}:{due_date}", recurring.id),
                source_type: "recurring".to_string(),
                recurring_id: Some(recurring.id),
                planned_expense_id: None,
                scheduled_date: due_date.clone(),
                name: recurring.name.clone(),
                amount: recurring.amount,
                currency: recurring.currency,
                tags: tags.clone(),
                is_subscription: recurring.is_subscription,
            });
        }
    }

    for (planned, tags) in planned_expenses {
        let date = planned.date.format("%Y-%m-%d").to_string();
        if date.as_str() <= today || date.as_str() > window_end.as_str() {
            continue;
        }

        if is_planned_expense_materialized(&planned_materialized, planned.id) {
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
