use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::extractor::AuthenticatedUser;
use crate::error::ApiError;
use crate::repos::users;
use crate::state::AppState;

pub async fn complete_onboarding(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<StatusCode, ApiError> {
    users::complete_onboarding(&state.db_pool, user.sub).await?;
    Ok(StatusCode::NO_CONTENT)
}
