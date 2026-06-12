use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::error::ApiError;
use crate::repos::{connection, tags};
use crate::state::AppState;

pub async fn list_tags(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<String>>, ApiError> {
    let mut conn = connection::user_connection(&state.db_pool, user.sub).await?;
    let names = tags::list_all_names(&mut conn, user.sub).await?;
    Ok(Json(names))
}
