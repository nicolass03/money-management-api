use std::net::IpAddr;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;
use tower_governor::key_extractor::{KeyExtractor, SmartIpKeyExtractor};

use crate::auth::jwt::AuthUser;
use crate::error::ApiError;
use crate::repos::users;
use crate::state::AppState;

/// Client IP behind Railway's edge proxy (which overwrites `X-Forwarded-For` / `X-Real-IP`),
/// falling back to the peer address. Same extractor as the per-IP request limiter.
fn client_ip(request: &Request<Body>) -> Option<IpAddr> {
    SmartIpKeyExtractor.extract(request).ok()
}

async fn authenticate(
    state: &AppState,
    token: &str,
    client_ip: Option<IpAddr>,
) -> Result<AuthUser, ApiError> {
    let user = match state.jwt_validator.validate(token).await {
        Ok(user) => user,
        Err(()) => {
            // Each failed attempt spends from the client's budget; once it's exhausted the client
            // gets 429 instead of 401 until the budget refills.
            let limited = state.rate_limit_enabled
                && client_ip.is_some_and(|ip| state.auth_failure_limiter.check_key(&ip).is_err());
            return Err(if limited {
                ApiError::TooManyRequests
            } else {
                ApiError::Unauthorized
            });
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
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let ip = client_ip(&request);
    let user = authenticate(&state, extract_bearer_token(&request)?, ip).await?;

    if !state.onboarded_users.contains(&user.sub) {
        if !users::is_onboarding_complete(&state.db_pool, user.sub).await? {
            return Err(ApiError::OnboardingRequired);
        }
        state.onboarded_users.insert(user.sub);
    }

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}

/// JWT + user row only — for routes that run before onboarding is complete.
pub async fn require_auth_onboarding_exempt(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let ip = client_ip(&request);
    let user = authenticate(&state, extract_bearer_token(&request)?, ip).await?;
    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}
