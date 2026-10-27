use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{MoneyContextQuery, MoneyContextResponse};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_money_context(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<MoneyContextQuery>,
) -> Result<Json<Arc<MoneyContextResponse>>, ApiError> {
    if query.force_refresh && state.rate_limit_enabled {
        state
            .force_refresh_limiter
            .check_key(&user.sub)
            .map_err(|_| ApiError::TooManyRequests)?;
    }

    let response = state
        .loader
        .money_context(user.sub, query.force_refresh)
        .await?;
    Ok(Json(response))
}
