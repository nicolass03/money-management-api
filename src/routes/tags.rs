use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::error::ApiError;
use crate::repos::tags;
use crate::state::AppState;

pub async fn list_tags(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<String>>, ApiError> {
    let mut conn = state.db_pool.get().await?;
    let names = tags::list_all_names(&mut conn, user.sub).await?;
    Ok(Json(names))
}
