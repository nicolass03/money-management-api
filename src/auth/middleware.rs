use axum::body::Body;
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::auth::jwt::{validate_token, AuthUser};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)?;

    let user = validate_token(token, &state.jwt_secret, &state.auth_user_email)
        .map_err(|()| ApiError::Unauthorized)?;

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}
