use chrono::Utc;
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel_async::AsyncConnection;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{
    accounts as accounts_repo, connection, expenses as expenses_repo, settings as settings_repo,
    tags as tags_repo,
};
use crate::services::accounts::{compute_balances, pick_funded_account, pick_richest_account};
use crate::services::currency::convert_amount;
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::pay_periods::{get_pay_dates_in_range, schedule_from_recurring};
use crate::state::DbPool;

fn is_recurring_due_unique_violation(error: &DieselError) -> bool {
    matches!(
        error,
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, info)
            if info.constraint_name().as_deref() == Some("expenses_recurring_due_unique")
    )
}

pub async fn charge_due_expenses_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
) -> Result<i32, ApiError> {
    let rates = get_exchange_rates(pool, false).await?;
    let settings = settings_repo::get_user_settings(pool, user_id).await?;
    let recurring_list = crate::repos::recurring_expenses::list_all(pool, user_id).await?;
    let mut materialized_ids =
        expenses_repo::get_materialized_recurring_ids_for_due_date(pool, user_id, date).await?;

    let display_currency = settings.display_currency;
    let now = Utc::now();
    let due_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest("invalid date".into()))?;

    // Account selection (requirement #6): charge a matching-currency account when it can cover the
    // expense, otherwise fall back to the display-currency account (allowed to go negative).
    // Balances are derived once and decremented in-memory as charges land so multiple same-day
    // charges see up-to-date balances.
    let accounts = accounts_repo::list_active(pool, user_id).await?;
    let mut balances = {
        let mut conn = connection::user_connection(pool, user_id).await?;
        compute_balances(&mut conn, user_id, &accounts, due_date).await?
    };
    let mut created = 0;

    for recurring in recurring_list {
        if let Some(last) = recurring.last_payment_date {
            let last_s = last.format("%Y-%m-%d").to_string();
            if date > last_s.as_str() {
                continue;
            }
        }

        let schedule = schedule_from_recurring(&recurring);
        let due_dates = get_pay_dates_in_range(&schedule, date, date);
        if due_dates.is_empty() || materialized_ids.contains(&recurring.id) {
            continue;
        }

        // Prefer a same-currency account with enough balance; charge it in the expense's own
        // currency (no conversion). Otherwise draw from the display-currency account, converting
        // the amount; if no account exists at all, fall back to the legacy converted insert.
        let (amount, currency, account_id) =
            match pick_funded_account(&accounts, &balances, recurring.currency, recurring.amount) {
                Some(account_id) => (recurring.amount, recurring.currency, Some(account_id)),
                None => {
                    let converted = if recurring.currency != display_currency {
                        convert_amount(recurring.amount, recurring.currency, display_currency, &rates)
                    } else {
                        recurring.amount
                    };
                    let fallback =
                        pick_richest_account(&accounts, &balances, display_currency);
                    (converted, display_currency, fallback)
                }
            };

        // Reflect the charge against the chosen account's running balance for later iterations.
        if let Some(id) = account_id {
            if let Some(balance) = balances.get_mut(&id) {
                *balance -= amount;
            }
        }

        let mut conn = connection::user_connection(pool, user_id).await?;
        let insert_result = conn
            .transaction(|conn| {
                Box::pin(async move {
                    let expense = expenses_repo::insert_expense(
                        conn,
                        user_id,
                        &recurring.name,
                        amount,
                        currency,
                        due_date,
                        None,
                        Some(recurring.id),
                        None,
                        None,
                        account_id,
                        false,
                        recurring.is_subscription,
                        now,
                    )
                    .await?;
                    tags_repo::copy_recurring_tags_to_expense(conn, recurring.id, expense.id)
                        .await?;
                    Ok::<_, diesel::result::Error>(expense)
                })
            })
            .await;

        match insert_result {
            Ok(_) => {
                materialized_ids.insert(recurring.id);
                created += 1;
            }
            Err(error) if is_recurring_due_unique_violation(&error) => {
                materialized_ids.insert(recurring.id);
            }
            Err(error) => return Err(ApiError::from(error)),
        }
    }

    if created > 0 {
        let mut conn = connection::user_connection(pool, user_id).await?;
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
