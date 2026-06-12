use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::auth::middleware::require_auth;
use crate::config::Config;
use crate::routes;
use crate::state::AppState;

pub fn build_app(config: &Config, state: AppState) -> Router {
    let request_timeout = config.request_timeout;
    let timeout_layer = middleware::from_fn(
        move |request: Request<Body>, next: Next| async move {
            match tokio::time::timeout(request_timeout, next.run(request)).await {
                Ok(response) => Ok(response),
                Err(_) => Err(StatusCode::REQUEST_TIMEOUT),
            }
        },
    );
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(
            config
                .cors_origins
                .iter()
                .map(|origin| origin.parse().expect("invalid CORS_ORIGIN entry"))
                .collect::<Vec<_>>(),
        );

    let common_layers = ServiceBuilder::new()
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(cors)
        .layer(timeout_layer)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new());

    let api = Router::new()
        .route("/settings", get(routes::settings::get_settings))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/health", get(routes::health::health))
        .nest("/api/v1", api)
        .layer(common_layers)
        .with_state(state)
}
