//! One-off backfill for the `projection_history` table.
//!
//! Computes and freezes every closed pay period for each user (or a single user via
//! `INIT_USER_ID`) and logs each period's values so the result can be verified. It is idempotent:
//! already-frozen periods are skipped. Set `DRY_RUN=true` to compute and log without writing, and
//! `REPLACE=true` to delete each user's existing rows and rebuild from scratch.
//!
//! Run with (verify first, then commit):
//!   RUST_LOG=info,money_management_api=debug DRY_RUN=true cargo run --bin init_projection_history
//!   RUST_LOG=info,money_management_api=debug cargo run --bin init_projection_history

use std::str::FromStr;

use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use money_management_api::config::Config;
use money_management_api::repos::users;
use money_management_api::services::projection_history::{sync_history, SyncOptions};
use money_management_api::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the crate-root .env exactly as the server does, regardless of cwd.
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path_override(env_path).ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,money_management_api=debug")),
        )
        .init();

    let dry_run = env_flag("DRY_RUN");
    let replace = env_flag("REPLACE");

    let config = Config::from_env()?;
    let state = AppState::new(&config).await?;
    let pool = state.db_pool.clone();

    let user_ids = match std::env::var("INIT_USER_ID") {
        Ok(raw) if !raw.trim().is_empty() => vec![Uuid::from_str(raw.trim())?],
        _ => users::list_user_ids(&pool).await?,
    };

    tracing::info!(
        users = user_ids.len(),
        dry_run,
        replace,
        "initializing projection history"
    );

    let mut total_computed = 0usize;
    let mut total_inserted = 0usize;
    for user_id in user_ids {
        match sync_history(&pool, user_id, SyncOptions { replace, dry_run }).await {
            Ok(report) => {
                tracing::info!(
                    %user_id,
                    computed = report.computed,
                    inserted = report.inserted,
                    "user projection history done"
                );
                total_computed += report.computed;
                total_inserted += report.inserted;
            }
            Err(error) => tracing::error!(%user_id, %error, "user projection history failed"),
        }
    }

    tracing::info!(
        total_computed,
        total_inserted,
        dry_run,
        "projection history initialization complete"
    );

    Ok(())
}

/// Reads a boolean-ish env flag: true for `1`/`true`/`yes` (case-insensitive), false otherwise.
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}
