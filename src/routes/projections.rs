use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{ProjectionsQuery, ProjectionsResponse};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_projections(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ProjectionsQuery>,
) -> Result<Json<ProjectionsResponse>, ApiError> {
    let response = state
        .loader
        .projections(user.sub, query.include_past)
        .await?;
    Ok(Json(response))
}
