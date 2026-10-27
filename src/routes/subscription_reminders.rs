use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::AsOfQuery;
use crate::error::ApiError;
use crate::models::SubscriptionReminderResponse;
use crate::repos::subscription_reminders as reminders_repo;
use crate::state::AppState;
use crate::validation::resolve_client_today;

/// Active cancellation reminders (undismissed, charge still upcoming) for the current user. Drives
/// the web banners.
pub async fn list_reminders(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<AsOfQuery>,
) -> Result<Json<Vec<SubscriptionReminderResponse>>, ApiError> {
    let today = resolve_client_today(query.as_of.as_deref())?;
    let reminders = reminders_repo::list_active(&state.db_pool, user.sub, today).await?;
    Ok(Json(reminders))
}

/// Permanently dismisses a reminder banner for the current user.
pub async fn dismiss_reminder(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let dismissed = reminders_repo::dismiss(&state.db_pool, user.sub, id).await?;
    if !dismissed {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "success": true })))
}
