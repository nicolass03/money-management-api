use axum::body::Body;
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn require_cron_secret(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let secret = state
        .cron_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)?;

    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value == &format!("Bearer {secret}"))
        .ok_or(ApiError::Unauthorized)?;

    let _ = token;
    Ok(next.run(request).await)
}
