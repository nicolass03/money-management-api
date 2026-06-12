mod app;
mod auth;
mod cache;
mod config;
mod jobs;
mod dto;
mod error;
mod models;
mod rate_limit;
mod repos;
mod routes;
mod schema;
mod services;
mod state;
mod validation;

use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

use crate::app::build_app;
use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load crate-root .env and override stale empty shell exports (e.g. after
    // `export DATABASE_URL="${DATABASE_URL//:6543/:5432}"` with an unset var).
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path_override(env_path).ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,money_management_api=debug")),
        )
        .init();

    let config = Config::from_env().map_err(|error| {
        eprintln!("configuration error: {error}");
        error
    })?;
    let addr = config.socket_addr().map_err(|error| {
        eprintln!("configuration error: {error}");
        error
    })?;

    let state = AppState::new(&config).await.map_err(|error| {
        eprintln!("startup error: {error}");
        error
    })?;

    jobs::daily_expenses::spawn_scheduler(state.db_pool.clone(), state.cache.clone(), &config);

    let app = build_app(&config, state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
