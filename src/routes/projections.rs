use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::ProjectionsResponse;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_projections(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ProjectionsResponse>, ApiError> {
    let response = state.loader.projections(user.sub).await?;
    Ok(Json(response))
}
