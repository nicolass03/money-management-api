use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::ApiError;
use crate::repos::users;
use crate::state::AppState;

pub async fn require_auth(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
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

    let user = match state.jwt_validator.validate(token).await {
        Ok(user) => user,
        Err(()) => {
            if state.rate_limit_enabled {
                let _ = state.auth_failure_limiter.check_key(&peer_addr.ip());
            }
            return Err(ApiError::Unauthorized);
        }
    };

    users::ensure_user_exists(&state.db_pool, user.sub, &user.email).await?;

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}
