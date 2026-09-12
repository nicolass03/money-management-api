use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::PatchSettingsRequest;
use crate::error::ApiError;
use crate::models::{UserSettingsResponse, UserSettingsRow};
use crate::repos::{income_schedules, settings as settings_repo};
use crate::services::pay_periods::add_months;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_language, parse_theme, regex_like_date,
    require_extra_spent_limit, today_iso,
};

pub async fn get_settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<UserSettingsResponse>, ApiError> {
    let row = state.loader.user_settings(user.sub).await?;
    let response = settings_response(&state.db_pool, user.sub, row).await?;
    Ok(Json(response))
}

pub async fn patch_settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<PatchSettingsRequest>,
) -> Result<Json<UserSettingsResponse>, ApiError> {
    let display_currency = match body.display_currency {
        Some(ref value) => Some(parse_currency(value)?),
        None => None,
    };
    let language = match body.language {
        Some(ref value) => Some(parse_language(value)?),
        None => None,
    };
    let theme = match body.theme {
        Some(ref value) => Some(parse_theme(value)?),
        None => None,
    };

    if let Some(Some(ref date)) = body.projection_start_date {
        if !regex_like_date(date) {
            return Err(ApiError::BadRequest("invalid projection start date".into()));
        }
    }
    if let Some(Some(ref date)) = body.projection_end_date {
        if !regex_like_date(date) {
            return Err(ApiError::BadRequest("invalid projection end date".into()));
        }
    }

    let projection_start_date = match body.projection_start_date {
        Some(Some(ref date)) => Some(Some(parse_date(date)?)),
        Some(None) => Some(None),
        None => None,
    };
    let projection_end_date = match body.projection_end_date {
        Some(Some(ref date)) => Some(Some(parse_date(date)?)),
        Some(None) => Some(None),
        None => None,
    };

    let server_today = parse_date(&today_iso())?;
    let as_of = match body.as_of.as_deref() {
        Some(date) => {
            if !regex_like_date(date) {
                return Err(ApiError::BadRequest("invalid as-of date".into()));
            }
            parse_date(date)?
        }
        None => server_today,
    };
    if (as_of - server_today).num_days().abs() > 1 {
        return Err(ApiError::BadRequest("invalid as-of date".into()));
    }

    if projection_start_date.is_some() || projection_end_date.is_some() {
        let current = state.loader.user_settings(user.sub).await?;
        let effective_start = projection_start_date
            .as_ref()
            .map_or(current.projection_start_date, |value| *value);
        let effective_end = projection_end_date
            .as_ref()
            .map_or(current.projection_end_date, |value| *value);

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

    state
        .cache
        .invalidate(InvalidationScope::SettingsChange, user.sub).await;

    // A new primary schedule, a moved projection start, or a different display currency all change
    // the period boundaries or denomination of the frozen history, so rebuild it. Revision was
    // already bumped above, so the next projections read picks up the rebuilt rows.
    let reinitialize_history = body.primary_schedule_id.is_some()
        || projection_start_date.is_some()
        || display_currency.is_some();
    if reinitialize_history {
        crate::services::projection_history::reinitialize_history(&state.db_pool, user.sub).await?;
    } else if projection_end_date.is_some() {
        // Extending an expired horizon may reveal closed periods that were never frozen.
        crate::services::projection_history::ensure_history(&state.db_pool, user.sub).await?;
    }

    let response = settings_response(&state.db_pool, user.sub, row).await?;
    Ok(Json(response))
}

fn validate_projection_dates(
    as_of: chrono::NaiveDate,
    start: Option<chrono::NaiveDate>,
    end: Option<chrono::NaiveDate>,
    validate_end_bounds: bool,
) -> Result<(), &'static str> {
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            return Err("projection end date cannot precede projection start date");
        }
    }

    if validate_end_bounds {
        let end = end.expect("an explicitly set projection end date is present");
        let max_end = chrono::NaiveDate::parse_from_str(
            &add_months(&as_of.format("%Y-%m-%d").to_string(), 24),
            "%Y-%m-%d",
        )
        .expect("calendar arithmetic returns a valid date");
        if end < as_of || end > max_end {
            return Err("projection end date must be between today and two years from today");
        }
    }

    Ok(())
}

async fn settings_response(
    pool: &crate::state::DbPool,
    user_id: uuid::Uuid,
    row: UserSettingsRow,
) -> Result<UserSettingsResponse, ApiError> {
    let primary_schedule = if let Some(schedule_id) = row.primary_schedule_id {
        income_schedules::find_by_id(pool, user_id, schedule_id).await?
    } else {
        None
    };
    Ok(UserSettingsResponse::from_row(row, primary_schedule))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
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
