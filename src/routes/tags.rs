use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_tags(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<String>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;
    let names = state
        .loader
        .tag_names(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(names))
}
