use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::PatchSettingsRequest;
use crate::error::ApiError;
use crate::models::UserSettingsResponse;
use crate::repos::settings as settings_repo;
use crate::state::AppState;
use crate::validation::{parse_currency, parse_date, regex_like_date};

pub async fn get_settings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<UserSettingsResponse>, ApiError> {
    let row = settings_repo::get_user_settings(&state.db_pool, user.sub).await?;
    Ok(Json(row.into()))
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

    let row = settings_repo::update_user_settings(
        &state.db_pool,
        user.sub,
        display_currency,
        body.primary_schedule_id,
        body.projection_initial_free_money,
        projection_start_date,
    )
    .await?;

    Ok(Json(row.into()))
}
