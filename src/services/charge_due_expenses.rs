use chrono::{NaiveDate, Utc};
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel_async::AsyncConnection;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{
    accounts as accounts_repo, connection, expenses as expenses_repo, recurring_expenses,
    settings as settings_repo, tags as tags_repo,
};
use crate::services::accounts::{
    compute_balances, pick_funded_account, pick_richest_account, pick_richest_any_currency,
    projected_accounts,
};
use crate::services::currency::convert_amount;
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::pay_periods::{occurs_on, schedule_from_recurring};
use crate::state::DbPool;

fn is_recurring_due_unique_violation(error: &DieselError) -> bool {
    matches!(
        error,
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, info)
            if info.constraint_name().as_deref() == Some("expenses_recurring_due_unique")
    )
}

/// Materializes the user's recurring expenses due on each of `dates` (oldest first, so
/// balance-based account picks see earlier charges). Templates, accounts, settings and rates are
/// loaded once for all dates, over one connection. Returns the number of charges created.
pub async fn charge_due_expenses(
    pool: &DbPool,
    user_id: Uuid,
    dates: &[NaiveDate],
) -> Result<i32, ApiError> {
    let rates = get_exchange_rates(pool, false).await?;
    let mut conn = connection::user_connection(pool, user_id).await?;
    let display_currency = settings_repo::get_user_settings_with_conn(&mut conn, user_id)
        .await?
        .display_currency;
    let recurring_list = recurring_expenses::list_all_with_conn(&mut conn, user_id).await?;
    let accounts = accounts_repo::list_active_with_conn(&mut conn, user_id).await?;
    // Automatic picks (steps 2–4 below) only use accounts counted in projections, unless every
    // account is excluded.
    let projected = projected_accounts(&accounts);
    let candidates = if projected.is_empty() { &accounts } else { &projected };
    let now = Utc::now();
    let mut created = 0;

    for &due_date in dates {
        let mut materialized_ids =
            expenses_repo::get_materialized_recurring_ids_for_due_date(&mut conn, user_id, due_date)
                .await?;
        // Balances are derived once per day and decremented in-memory as charges land, so
        // multiple same-day charges see up-to-date balances.
        let mut balances = compute_balances(&mut conn, user_id, &accounts, due_date).await?;

        for recurring in &recurring_list {
            // Never materialize an occurrence dated before the template was last saved: creating
            // or editing a template doesn't backfill history, even when the daily job catches up
            // on days it missed. (`occurs_on` already respects the last payment date.)
            if due_date < recurring.updated_at.date_naive()
                || materialized_ids.contains(&recurring.id)
                || !occurs_on(&schedule_from_recurring(recurring), due_date)
            {
                continue;
            }

            // Account selection order:
            //  1. An explicitly pinned, still-active account — charged in its (== the template's)
            //     currency even if it can't cover the charge (allowed to go negative), honoring
            //     the user's choice.
            //  2. Otherwise pick by currency among the candidates (projected accounts first): a
            //     same-currency account that can cover the charge.
            //  3. Else the richest display-currency account (converted, may go negative).
            //  4. Else the richest account in ANY currency (by converted value) — so a charge is
            //     never stranded off-book when the user has no display-currency account.
            //  5. Only when the user has no active account at all does it stay unattributed.
            let pinned = recurring
                .account_id
                .filter(|id| accounts.iter().any(|a| a.id == *id));
            let convert_to = |currency| {
                convert_amount(recurring.amount, recurring.currency, currency, &rates)
            };
            let (amount, currency, account_id) = if let Some(id) = pinned {
                (recurring.amount, recurring.currency, Some(id))
            } else if let Some(id) =
                pick_funded_account(candidates, &balances, recurring.currency, recurring.amount)
            {
                (recurring.amount, recurring.currency, Some(id))
            } else if let Some(id) = pick_richest_account(candidates, &balances, display_currency) {
                (convert_to(display_currency), display_currency, Some(id))
            } else if let Some(account) = pick_richest_any_currency(candidates, &balances, &rates) {
                (convert_to(account.currency), account.currency, Some(account.id))
            } else {
                (convert_to(display_currency), display_currency, None)
            };

            // The transaction closure must own what it uses (it can't borrow the template list
            // across the pooled connection's lifetime).
            let (recurring_id, name, is_subscription) =
                (recurring.id, recurring.name.clone(), recurring.is_subscription);
            let insert_result = conn
                .transaction(|conn| {
                    Box::pin(async move {
                        let expense = expenses_repo::insert_expense(
                            conn,
                            expenses_repo::NewExpense {
                                user_id,
                                name: &name,
                                amount,
                                currency,
                                date: due_date,
                                scheduled_date: None,
                                recurring_id: Some(recurring_id),
                                planned_expense_id: None,
                                budget_id: None,
                                account_id,
                                amount_overridden: false,
                                is_subscription,
                                created_at: now,
                            },
                        )
                        .await?;
                        tags_repo::copy_recurring_tags_to_expense(conn, recurring_id, expense.id)
                            .await?;
                        Ok::<_, diesel::result::Error>(expense)
                    })
                })
                .await;

            match insert_result {
                Ok(_) => {
                    created += 1;
                    // Reflect the charge in the chosen account's running balance.
                    if let Some(balance) = account_id.and_then(|id| balances.get_mut(&id)) {
                        *balance -= i64::from(amount);
                    }
                }
                Err(error) if is_recurring_due_unique_violation(&error) => {}
                Err(error) => return Err(ApiError::from(error)),
            }
            materialized_ids.insert(recurring.id);
        }
    }

    if created > 0 {
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
