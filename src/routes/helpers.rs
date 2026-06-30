use uuid::Uuid;

use crate::error::ApiError;
use crate::models::CurrencyCode;
use crate::repos::{accounts, income_schedules, settings};
use crate::services::pay_periods::{get_period_containing, schedule_from_income, PayPeriod};
use crate::state::DbPool;

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
    match account_id {
        Some(id) => {
            let account = accounts::find_by_id(pool, user_id, id)
                .await?
                .ok_or_else(|| ApiError::BadRequest("account not found".into()))?;
            if account.archived_at.is_some() {
                return Err(ApiError::BadRequest("account is archived".into()));
            }
            Ok((Some(id), account.currency))
        }
        None => Ok((None, fallback_currency)),
    }
}

pub async fn get_current_pay_period(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Option<PayPeriod>, ApiError> {
    let user_settings = settings::get_user_settings(pool, user_id).await?;
    let Some(schedule_id) = user_settings.primary_schedule_id else {
        return Ok(None);
    };
    let Some(schedule) = income_schedules::find_by_id(pool, user_id, schedule_id).await? else {
        return Ok(None);
    };
    let today = crate::validation::today_iso();
    Ok(Some(get_period_containing(
        &schedule_from_income(&schedule),
        &today,
    )))
}
