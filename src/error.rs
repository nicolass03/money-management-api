use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use diesel::result::Error as DieselError;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("onboarding_required")]
    OnboardingRequired,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("database error: {0}")]
    Database(#[from] DieselError),
    #[error("pool error: {0}")]
    Pool(#[from] diesel_async::pooled_connection::bb8::RunError),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            ApiError::OnboardingRequired => (StatusCode::FORBIDDEN, self.to_string()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            ApiError::Database(DieselError::NotFound) => {
                (StatusCode::NOT_FOUND, "not found".to_string())
            }
            ApiError::Database(error) => {
                tracing::error!(%error, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            ApiError::Pool(error) => {
                tracing::error!(%error, "connection pool error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            ApiError::Internal(error) => {
                tracing::error!(%error, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}
