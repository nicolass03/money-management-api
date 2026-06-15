use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::auth::jwt::AuthUser;
use crate::error::ApiError;
use crate::repos::users;
use crate::state::AppState;

async fn authenticate(
    state: &AppState,
    peer_addr: SocketAddr,
    token: &str,
) -> Result<AuthUser, ApiError> {
    let user = match state.jwt_validator.validate(token).await {
        Ok(user) => user,
        Err(()) => {
            if state.rate_limit_enabled {
                let _ = state.auth_failure_limiter.check_key(&peer_addr.ip());
            }
            return Err(ApiError::Unauthorized);
        }
    };

    if !state.known_users.contains(&user.sub) {
        users::ensure_user_exists(&state.db_pool, user.sub, &user.email).await?;
        state.known_users.insert(user.sub);
    }

    Ok(user)
}

fn extract_bearer_token(request: &Request<Body>) -> Result<&str, ApiError> {
    request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)
}

pub async fn require_auth(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer_token(&request)?;
    let user = authenticate(&state, peer_addr, token).await?;

    if !users::is_onboarding_complete(&state.db_pool, user.sub).await? {
        return Err(ApiError::OnboardingRequired);
    }

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}

/// JWT + user row only — for routes that run before onboarding is complete.
pub async fn require_auth_onboarding_exempt(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer_token(&request)?;
    let user = authenticate(&state, peer_addr, token).await?;
    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}
