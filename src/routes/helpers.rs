use chrono::NaiveDate;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, ExpenseRow};
use crate::repos::{accounts, budgets, connection};
use crate::services::accounts::{compute_balances, pick_funded_account, pick_richest_account};
use crate::services::pay_periods::{get_period_containing, schedule_from_income, PayPeriod};
use crate::services::projection_history::{self, HistorySync};
use crate::state::{AppState, DbPool};

/// Validates an optional account selection and resolves the currency a row should be stored in.
/// When an account is given it must belong to the user and be active; the row's currency then
/// follows the account's currency (currency-follows-account, so derived balances never need
/// conversion). When no account is given, the caller's submitted currency is used as-is.
pub async fn resolve_account(
    pool: &DbPool,
    user_id: Uuid,
    account_id: Option<Uuid>,
    fallback_currency: CurrencyCode,
) -> Result<(Option<Uuid>, CurrencyCode), ApiError> {
    resolve_account_for_update(pool, user_id, account_id, None, fallback_currency).await
}

/// Like [`resolve_account`], but permits keeping an account that is now archived as long as it is
/// the one already attached to the row being updated (`current_account_id`). This lets a user edit
/// a row whose account was archived without being forced to reassign it, while still forbidding a
/// *new* assignment of an archived account.
pub async fn resolve_account_for_update(
    pool: &DbPool,
    user_id: Uuid,
    account_id: Option<Uuid>,
    current_account_id: Option<Uuid>,
    fallback_currency: CurrencyCode,
) -> Result<(Option<Uuid>, CurrencyCode), ApiError> {
    match account_id {
        Some(id) => {
            let account = accounts::find_by_id(pool, user_id, id)
                .await?
                .ok_or_else(|| ApiError::BadRequest("account not found".into()))?;
            let unchanged = current_account_id == Some(id);
            if account.archived_at.is_some() && !unchanged {
                return Err(ApiError::BadRequest("account is archived".into()));
            }
            // Currency-follows-account is the invariant balances rely on (amounts are summed with no
            // FX conversion). The amount must already be denominated in the account's currency, so
            // reject a mismatched submitted currency rather than silently reinterpret the amount's
            // magnitude in a different currency (which would corrupt the balance).
            if fallback_currency != account.currency {
                return Err(ApiError::BadRequest(
                    "amount currency must match the selected account's currency".into(),
                ));
            }
            Ok((Some(id), account.currency))
        }
        None => Ok((None, fallback_currency)),
    }
}

/// Account a one-off payment in `currency` draws from: `preferred` while it is still active; else
/// a same-currency account that covers `amount`; else the richest same-currency account (may go
/// negative); `None` when the user holds no active account in that currency. Keeps
/// currency-follows-account, since every candidate is in the payment's currency.
pub async fn pick_payment_account(
    pool: &DbPool,
    user_id: Uuid,
    preferred: Option<Uuid>,
    currency: CurrencyCode,
    amount: i32,
    as_of: NaiveDate,
) -> Result<Option<Uuid>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let accounts_list = accounts::list_active_with_conn(&mut conn, user_id).await?;
    if let Some(id) = preferred.filter(|id| accounts_list.iter().any(|a| a.id == *id)) {
        return Ok(Some(id));
    }
    let balances = compute_balances(&mut conn, user_id, &accounts_list, as_of).await?;
    Ok(pick_funded_account(&accounts_list, &balances, currency, amount)
        .or_else(|| pick_richest_account(&accounts_list, &balances, currency)))
}

/// The primary schedule's pay period containing `today`, if a primary schedule is set.
pub async fn get_current_pay_period(
    state: &AppState,
    user_id: Uuid,
    today: NaiveDate,
) -> Result<Option<PayPeriod>, ApiError> {
    let settings = state.loader.user_settings(user_id).await?;
    let schedule = state.loader.primary_schedule(user_id, &settings).await?;
    Ok(schedule.map(|schedule| get_period_containing(&schedule_from_income(&schedule), today)))
}

/// Finishes every committed write: re-freezes the projection history the write may have touched
/// (`history`), then invalidates the user's caches. History goes first so a projection read racing
/// the write can't cache stale frozen rows under the new revision — the invalidation evicts
/// anything cached in between. A failed re-freeze doesn't fail the (already committed) write: the
/// frozen rows are dropped instead, so projections fall back to a full live computation until the
/// daily job freezes them again, rather than serving stale aggregates.
pub async fn finish_write(state: &AppState, user_id: Uuid, history: Option<HistorySync>) {
    let refreshed = match history {
        None => Ok(()),
        // Settings writes rebuild instead, so the cached primary schedule id is current here.
        Some(HistorySync::From(date)) => match state.loader.user_settings(user_id).await {
            Ok(settings) => {
                let pool = &state.db_pool;
                let schedule_id = settings.primary_schedule_id;
                projection_history::refresh_history_for_date(pool, user_id, schedule_id, date).await
            }
            Err(error) => Err(error),
        },
        Some(sync) => projection_history::sync_history(&state.db_pool, user_id, sync, false)
            .await
            .map(|_| ()),
    };
    if let Err(error) = refreshed {
        tracing::error!(%user_id, %error, "projection history refresh failed; clearing frozen rows");
        if let Err(error) = projection_history::clear_history(&state.db_pool, user_id).await {
            tracing::error!(%user_id, %error, "clearing projection history failed");
        }
    }
    state.cache.invalidate(user_id).await;
}

/// Earliest date whose projection period an expense affects, for re-freezing history: its own date,
/// or — for spend against a dated budget, which projections show as the budget's line on its end
/// date — the earlier of that and the budget's end date. If the budget can't be read, falls back to
/// the earliest possible date (a full re-freeze) rather than risk leaving frozen rows stale.
pub async fn expense_history_date(state: &AppState, user_id: Uuid, expense: &ExpenseRow) -> NaiveDate {
    let Some(budget_id) = expense.budget_id else {
        return expense.date;
    };
    match budgets::find_by_id(&state.db_pool, user_id, budget_id).await {
        Ok(budget) => budget
            .and_then(|budget| budget.end_date)
            .map_or(expense.date, |end| end.min(expense.date)),
        Err(error) => {
            tracing::error!(%user_id, %error, "budget lookup for history refresh failed");
            NaiveDate::MIN
        }
    }
}
