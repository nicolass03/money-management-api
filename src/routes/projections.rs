use std::sync::Arc;

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
) -> Result<Json<Arc<ProjectionsResponse>>, ApiError> {
    let response = state
        .loader
        .projections(user.sub, query.include_past, query.as_of.as_deref())
        .await?;
    Ok(Json(response))
}
