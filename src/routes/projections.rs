use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{ProjectionPeriodItemsQuery, ProjectionsQuery, ProjectionsResponse};
use crate::error::ApiError;
use crate::services::projections::ProjectionRow;
use crate::state::AppState;
use crate::validation::parse_date;

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

/// Expense-item breakdown for a single (usually past) projection period, identified by its
/// closing pay date. Backs the UI expanding a frozen history row, which stores aggregates only.
pub async fn get_projection_period_items(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ProjectionPeriodItemsQuery>,
) -> Result<Json<Arc<ProjectionRow>>, ApiError> {
    parse_date(&query.pay_date).map_err(|_| ApiError::BadRequest("invalid pay date".into()))?;
    let row = state
        .loader
        .projection_period_items(user.sub, &query.pay_date, query.as_of.as_deref())
        .await?;
    Ok(Json(row))
}
