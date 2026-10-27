use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{AsOfQuery, CreateAccountRequest, UpdateAccountRequest};
use crate::error::ApiError;
use crate::models::{account_to_response, AccountResponse};
use crate::repos::accounts as accounts_repo;
use crate::repos::connection;
use crate::routes::helpers::finish_write;
use crate::services::accounts::compute_balances;
use crate::services::projection_history::HistorySync;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_optional_name, require_initial_amount, resolve_client_today,
};

/// Active accounts with their balances as of `as_of` (the client's local today).
async fn list_with_balances(
    state: &AppState,
    user_id: Uuid,
    as_of: NaiveDate,
) -> Result<Vec<AccountResponse>, ApiError> {
    let mut conn = connection::user_connection(&state.db_pool, user_id).await?;
    let accounts = accounts_repo::list_active_with_conn(&mut conn, user_id).await?;
    let balances = compute_balances(&mut conn, user_id, &accounts, as_of).await?;
    Ok(accounts
        .into_iter()
        .map(|account| {
            // compute_balances returns an entry for every account passed in; 0 is only a defensive
            // default and never actually used.
            let balance = balances.get(&account.id).copied().unwrap_or(0);
            account_to_response(account, balance)
        })
        .collect())
}

/// Rebuilds the frozen projection history and invalidates caches: the projection seed is the sum
/// of account starting balances, and excluding an account drops its rows from every period, so any
/// account create/edit/archive changes every frozen cumulative value. Mirrors the settings patch
/// path.
async fn after_account_change(state: &AppState, user_id: Uuid) {
    finish_write(state, user_id, Some(HistorySync::Rebuild)).await;
}

pub async fn list_accounts(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<AsOfQuery>,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    let as_of = resolve_client_today(query.as_of.as_deref())?;
    Ok(Json(list_with_balances(&state, user.sub, as_of).await?))
}

pub async fn create_account(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let name = parse_optional_name(body.name.as_deref())?;
    let currency = parse_currency(&body.currency)?;
    let initial_amount = require_initial_amount(body.initial_amount)?;

    let row = accounts_repo::create(
        &state.db_pool,
        user.sub,
        name.as_deref(),
        currency,
        initial_amount,
        body.exclude_from_projections,
    )
    .await?;
    after_account_change(&state, user.sub).await;
    // A brand-new account has no activity yet, so balance == initial amount.
    Ok(Json(account_to_response(row, i64::from(initial_amount))))
}

pub async fn update_account(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Query(query): Query<AsOfQuery>,
    Json(body): Json<UpdateAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let as_of = resolve_client_today(query.as_of.as_deref())?;
    // Currency is immutable after creation (see repos::accounts::update); the request's currency
    // field is ignored. Only name, initial amount and the projection flag are updated.
    let name = parse_optional_name(body.name.as_deref())?;
    let initial_amount = require_initial_amount(body.initial_amount)?;

    accounts_repo::update(
        &state.db_pool,
        user.sub,
        id,
        name.as_deref(),
        initial_amount,
        body.exclude_from_projections,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    after_account_change(&state, user.sub).await;

    // Return the updated account with its freshly recomputed balance.
    let accounts = list_with_balances(&state, user.sub, as_of).await?;
    accounts
        .into_iter()
        .find(|a| a.id == id)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn delete_account(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Refuse to archive the user's last active account: with zero accounts, new expenses/income
    // are saved with a NULL account and fall out of every balance (an off-book state).
    if accounts_repo::count_active(&state.db_pool, user.sub).await? <= 1 {
        return Err(ApiError::BadRequest(
            "cannot archive your only account".into(),
        ));
    }
    let archived = accounts_repo::archive(&state.db_pool, user.sub, id).await?;
    if !archived {
        return Err(ApiError::NotFound);
    }
    after_account_change(&state, user.sub).await;
    Ok(Json(serde_json::json!({ "success": true })))
}
