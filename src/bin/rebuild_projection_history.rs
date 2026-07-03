//! One-off maintenance binary: rebuild the frozen `projection_history` table.
//!
//! Existing frozen rows were computed with the old projection seed that double-counted the opening
//! balance — accounts' initial amounts PLUS the now-removed `user_settings.projection_initial_free_money`.
//! The seed is now accounts-only (see `services::projection_history::load_projection_inputs` and
//! `cache::loader`), but `ensure_history` only *appends* newly-closed periods, so already-frozen
//! rows keep their stale (doubled) `cumulative`. This binary deletes each user's frozen rows and
//! re-freezes them from scratch with the corrected seed (`replace: true`).
//!
//! Usage (from the crate root, with the same `.env` / `DATABASE_URL` the server uses):
//!   cargo run --bin rebuild_projection_history                # rebuild every user
//!   cargo run --bin rebuild_projection_history -- <user_id>   # rebuild a single user
//!   cargo run --bin rebuild_projection_history -- --dry-run   # compute + log only, no writes
//!
//! It is idempotent: a rebuild always fully replaces the primary schedule's rows, so it is safe to
//! run more than once. Do a `--dry-run` first to review the recomputed cumulatives in the logs.

use money_management_api::config::Config;
use money_management_api::repos::users;
use money_management_api::services::projection_history::{sync_history, SyncOptions};
use money_management_api::state::AppState;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Mirror the server's startup: load the crate-root .env, then read config from the environment.
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path_override(env_path).ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,money_management_api=info")),
        )
        .init();

    // Args: an optional `--dry-run` flag and an optional single target user id.
    let mut dry_run = false;
    let mut target_user: Option<Uuid> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            other => {
                target_user =
                    Some(Uuid::parse_str(other).map_err(|_| format!("invalid user id: {other}"))?);
            }
        }
    }

    let config = Config::from_env()?;
    // AppState builds the TLS-backed connection pool exactly as the server does. (It also inits the
    // JWT validator, which is unused here but harmless.)
    let state = AppState::new(&config).await?;
    let pool = state.db_pool;

    let user_ids = match target_user {
        Some(id) => vec![id],
        None => users::list_user_ids(&pool).await?,
    };

    tracing::info!(users = user_ids.len(), dry_run, "rebuilding projection history");

    let mut total_computed = 0usize;
    let mut total_inserted = 0usize;
    let mut failures = 0usize;
    for user_id in user_ids {
        match sync_history(&pool, user_id, SyncOptions { replace: true, dry_run }).await {
            Ok(report) => {
                tracing::info!(
                    %user_id,
                    computed = report.computed,
                    inserted = report.inserted,
                    "rebuilt user"
                );
                total_computed += report.computed;
                total_inserted += report.inserted;
            }
            Err(error) => {
                failures += 1;
                tracing::error!(%user_id, %error, "rebuild failed for user");
            }
        }
    }

    tracing::info!(
        total_computed,
        total_inserted,
        failures,
        dry_run,
        "projection history rebuild complete"
    );

    if failures > 0 {
        return Err(format!("{failures} user(s) failed to rebuild").into());
    }
    Ok(())
}
