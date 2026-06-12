use axum::extract::State;
use axum::Json;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::error::ApiError;
use crate::models::{UserSettingsResponse, UserSettingsRow};
use crate::schema::user_settings;
use crate::state::AppState;

const SETTINGS_ID: i32 = 1;

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<UserSettingsResponse>, ApiError> {
    let mut conn = state.db_pool.get().await?;

    let row = user_settings::table
        .find(SETTINGS_ID)
        .select(UserSettingsRow::as_select())
        .first(&mut conn)
        .await?;

    Ok(Json(row.into()))
}
