use std::collections::HashSet;

use uuid::Uuid;

use crate::models::ExpenseRow;

pub fn recurring_due_date(expense: &ExpenseRow) -> String {
    expense
        .scheduled_date
        .unwrap_or(expense.date)
        .format("%Y-%m-%d")
        .to_string()
}

pub fn materialized_recurring_key(recurring_id: Uuid, due_date: &str) -> String {
    format!("{recurring_id}:{due_date}")
}

pub fn build_recurring_materialized_set(expense_list: &[ExpenseRow]) -> HashSet<String> {
    let mut materialized = HashSet::new();
    for expense in expense_list {
        if let Some(recurring_id) = expense.recurring_id {
            materialized.insert(materialized_recurring_key(
                recurring_id,
                &recurring_due_date(expense),
            ));
        }
    }
    materialized
}

pub fn build_planned_materialized_set(expense_list: &[ExpenseRow]) -> HashSet<Uuid> {
    expense_list
        .iter()
        .filter_map(|expense| expense.planned_expense_id)
        .collect()
}

pub fn is_recurring_occurrence_materialized(
    materialized: &HashSet<String>,
    recurring_id: Uuid,
    due_date: &str,
) -> bool {
    materialized.contains(&materialized_recurring_key(recurring_id, due_date))
}

pub fn is_planned_expense_materialized(
    materialized: &HashSet<Uuid>,
    planned_expense_id: Uuid,
) -> bool {
    materialized.contains(&planned_expense_id)
}
