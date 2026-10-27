use std::collections::HashSet;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::models::ExpenseRow;

/// The occurrence a recurring charge settles: its scheduled date when paid early, else its date.
pub fn recurring_due_date(expense: &ExpenseRow) -> NaiveDate {
    expense.scheduled_date.unwrap_or(expense.date)
}

/// `(recurring_id, due date)` of every materialized recurring occurrence.
pub fn build_recurring_materialized_set<'a>(
    expense_list: impl IntoIterator<Item = &'a ExpenseRow>,
) -> HashSet<(Uuid, NaiveDate)> {
    expense_list
        .into_iter()
        .filter_map(|expense| {
            expense
                .recurring_id
                .map(|recurring_id| (recurring_id, recurring_due_date(expense)))
        })
        .collect()
}

pub fn build_planned_materialized_set<'a>(
    expense_list: impl IntoIterator<Item = &'a ExpenseRow>,
) -> HashSet<Uuid> {
    expense_list
        .into_iter()
        .filter_map(|expense| expense.planned_expense_id)
        .collect()
}
