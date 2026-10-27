use axum::extract::State;
use axum::Json;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::PatchSettingsRequest;
use crate::error::ApiError;
use crate::models::{UserSettingsResponse, UserSettingsRow};
use crate::repos::{income_schedules, settings as settings_repo};
use crate::routes::helpers::finish_write;
use crate::services::pay_periods::add_months;
use crate::services::projection_history::HistorySync;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_language, parse_theme, require_extra_spent_limit,
    resolve_client_today,
};

pub async fn get_settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<UserSettingsResponse>, ApiError> {
    let row = state.loader.user_settings(user.sub).await?;
    Ok(Json(settings_response(&state, user.sub, &row).await?))
}

/// `Some(None)` clears a nullable date, `Some(Some(_))` sets it, `None` leaves it untouched.
fn parse_optional_date_patch(
    value: &Option<Option<String>>,
    error: &str,
) -> Result<Option<Option<NaiveDate>>, ApiError> {
    match value {
        Some(Some(date)) => parse_date(date)
            .map(|date| Some(Some(date)))
            .map_err(|_| ApiError::BadRequest(error.into())),
        Some(None) => Ok(Some(None)),
        None => Ok(None),
    }
}

pub async fn patch_settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<PatchSettingsRequest>,
) -> Result<Json<UserSettingsResponse>, ApiError> {
    let display_currency = body.display_currency.as_deref().map(parse_currency).transpose()?;
    let language = body.language.as_deref().map(parse_language).transpose()?;
    let theme = body.theme.as_deref().map(parse_theme).transpose()?;
    let projection_start_date =
        parse_optional_date_patch(&body.projection_start_date, "invalid projection start date")?;
    let projection_end_date =
        parse_optional_date_patch(&body.projection_end_date, "invalid projection end date")?;
    let as_of = resolve_client_today(body.as_of.as_deref())?;

    if projection_start_date.is_some() || projection_end_date.is_some() {
        let current = state.loader.user_settings(user.sub).await?;
        let effective_start = projection_start_date.unwrap_or(current.projection_start_date);
        let effective_end = projection_end_date.unwrap_or(current.projection_end_date);

        validate_projection_dates(
            as_of,
            effective_start,
            effective_end,
            matches!(projection_end_date, Some(Some(_))),
        )
        .map_err(|message| ApiError::BadRequest(message.into()))?;
    }

    if let Some(Some(schedule_id)) = body.primary_schedule_id {
        income_schedules::find_by_id(&state.db_pool, user.sub, schedule_id)
            .await?
            .ok_or(ApiError::NotFound)?;
    }

    let extra_spent_limit = match body.extra_spent_limit {
        Some(Some(value)) => Some(Some(require_extra_spent_limit(value)?)),
        Some(None) => Some(None),
        None => None,
    };

    let row = settings_repo::update_user_settings(
        &state.db_pool,
        user.sub,
        settings_repo::UserSettingsPatch {
            display_currency,
            language,
            primary_schedule_id: body.primary_schedule_id,
            projection_start_date,
            projection_end_date,
            extra_spent_limit,
            theme,
        },
    )
    .await?;

    // A new primary schedule, a moved projection start, or a different display currency all change
    // the period boundaries or denomination of the frozen history, so rebuild it. Extending an
    // expired horizon may reveal closed periods that were never frozen, so append those.
    let history = if body.primary_schedule_id.is_some()
        || projection_start_date.is_some()
        || display_currency.is_some()
    {
        Some(HistorySync::Rebuild)
    } else if projection_end_date.is_some() {
        Some(HistorySync::Append)
    } else {
        None
    };
    finish_write(&state, user.sub, history).await;

    Ok(Json(settings_response(&state, user.sub, &row).await?))
}

fn validate_projection_dates(
    as_of: NaiveDate,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    validate_end_bounds: bool,
) -> Result<(), &'static str> {
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            return Err("projection end date cannot precede projection start date");
        }
    }

    if validate_end_bounds {
        let end = end.expect("an explicitly set projection end date is present");
        if end < as_of || end > add_months(as_of, 24) {
            return Err("projection end date must be between today and two years from today");
        }
    }

    Ok(())
}

/// The settings response, embedding the primary schedule (from the cached schedule list).
async fn settings_response(
    state: &AppState,
    user_id: Uuid,
    row: &UserSettingsRow,
) -> Result<UserSettingsResponse, ApiError> {
    let primary_schedule = state.loader.primary_schedule(user_id, row).await?;
    Ok(UserSettingsResponse::from_row(row.clone(), primary_schedule))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn projection_end_accepts_today_and_two_year_boundary() {
        let today = date("2026-09-12");
        assert!(validate_projection_dates(today, None, Some(today), true).is_ok());
        assert!(validate_projection_dates(
            today,
            None,
            Some(date("2028-09-12")),
            true,
        )
        .is_ok());
    }

    #[test]
    fn projection_end_rejects_out_of_range_and_before_start() {
        let today = date("2026-09-12");
        assert!(validate_projection_dates(
            today,
            None,
            Some(date("2026-09-11")),
            true,
        )
        .is_err());
        assert!(validate_projection_dates(
            today,
            None,
            Some(date("2028-09-13")),
            true,
        )
        .is_err());
        assert!(validate_projection_dates(
            today,
            Some(date("2027-01-02")),
            Some(date("2027-01-01")),
            true,
        )
        .is_err());
    }
}
