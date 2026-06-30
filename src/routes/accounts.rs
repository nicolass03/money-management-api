use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{CreateAccountRequest, UpdateAccountRequest};
use crate::error::ApiError;
use crate::models::{account_to_response, AccountResponse};
use crate::repos::accounts as accounts_repo;
use crate::repos::connection;
use crate::services::accounts::compute_balances;
use crate::state::AppState;
use crate::validation::{parse_currency, parse_optional_name, require_initial_amount, today_iso};

async fn list_with_balances(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<AccountResponse>, ApiError> {
    let accounts = accounts_repo::list_active(&state.db_pool, user_id).await?;
    let as_of = crate::validation::parse_date(&today_iso())?;
    let mut conn = connection::user_connection(&state.db_pool, user_id).await?;
    let balances = compute_balances(&mut conn, user_id, &accounts, as_of).await?;
    Ok(accounts
        .into_iter()
        .map(|account| {
            let balance = balances.get(&account.id).copied().unwrap_or(account.initial_amount);
            account_to_response(account, balance)
        })
        .collect())
}

pub async fn list_accounts(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    Ok(Json(list_with_balances(&state, user.sub).await?))
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
    )
    .await?;
    state
        .cache
        .invalidate(InvalidationScope::AccountChange, user.sub)
        .await;
    // A brand-new account has no activity yet, so balance == initial amount.
    Ok(Json(account_to_response(row, initial_amount)))
}

pub async fn update_account(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let name = parse_optional_name(body.name.as_deref())?;
    let currency = parse_currency(&body.currency)?;
    let initial_amount = require_initial_amount(body.initial_amount)?;

    accounts_repo::update(
        &state.db_pool,
        user.sub,
        id,
        name.as_deref(),
        currency,
        initial_amount,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::AccountChange, user.sub)
        .await;

    // Return the updated account with its freshly recomputed balance.
    let accounts = list_with_balances(&state, user.sub).await?;
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
    let archived = accounts_repo::archive(&state.db_pool, user.sub, id).await?;
    if !archived {
        return Err(ApiError::NotFound);
    }
    state
        .cache
        .invalidate(InvalidationScope::AccountChange, user.sub)
        .await;
    Ok(Json(serde_json::json!({ "success": true })))
}
