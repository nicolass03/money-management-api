use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::auth::jwt::AuthUser;
use crate::error::ApiError;

pub struct AuthenticatedUser(pub AuthUser);

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .map(AuthenticatedUser)
            .ok_or(ApiError::Unauthorized)
    }
}
