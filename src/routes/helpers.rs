use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{income_schedules, settings};
use crate::services::pay_periods::{get_period_containing, schedule_from_income, PayPeriod};
use crate::state::DbPool;

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
