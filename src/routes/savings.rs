use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::error::ApiError;
use crate::models::SavingResponse;
use crate::repos::savings as savings_repo;
use crate::state::AppState;

pub async fn list_savings(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<SavingResponse>>, ApiError> {
    let rows = savings_repo::list_all(&state.db_pool, user.sub).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}
