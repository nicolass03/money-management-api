mod app;
mod auth;
mod config;
mod error;
mod models;
mod routes;
mod schema;
mod state;

use tracing_subscriber::EnvFilter;

use crate::app::build_app;
use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

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

    let app = build_app(&config, state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(listener, app)
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
