use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::PatchSettingsRequest;
use crate::error::ApiError;
use crate::models::{UserSettingsResponse, UserSettingsRow};
use crate::repos::{income_schedules, settings as settings_repo};
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, regex_like_date, require_projection_free_money,
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

    if let Some(Some(ref date)) = body.projection_start_date {
        if !regex_like_date(date) {
            return Err(ApiError::BadRequest("invalid projection start date".into()));
        }
    }

    let projection_start_date = match body.projection_start_date {
        Some(Some(ref date)) => Some(Some(parse_date(date)?)),
        Some(None) => Some(None),
        None => None,
    };

    if let Some(Some(schedule_id)) = body.primary_schedule_id {
        income_schedules::find_by_id(&state.db_pool, user.sub, schedule_id)
            .await?
            .ok_or(ApiError::NotFound)?;
    }

    let projection_initial_free_money = match body.projection_initial_free_money {
        Some(value) => Some(require_projection_free_money(value)?),
        None => None,
    };

    let row = settings_repo::update_user_settings(
        &state.db_pool,
        user.sub,
        display_currency,
        body.primary_schedule_id,
        projection_initial_free_money,
        projection_start_date,
    )
    .await?;

    state
        .cache
        .invalidate(InvalidationScope::SettingsChange, user.sub);

    let response = settings_response(&state.db_pool, user.sub, row).await?;
    Ok(Json(response))
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
