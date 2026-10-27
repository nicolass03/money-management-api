use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{AsOfQuery, CreateAccountRequest, UpdateAccountRequest};
use crate::error::ApiError;
use crate::models::{
    account_to_response, AccountResponse, AccountRow, InterestCreditResponse, InterestRateType,
};
use crate::repos::accounts::{self as accounts_repo, AccountInterest};
use crate::repos::{connection, income as income_repo};
use crate::routes::helpers::finish_write;
use crate::services::accounts::compute_balances;
use crate::services::projection_history::HistorySync;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_interest_rate_type, parse_optional_name, require_initial_amount,
    require_interest_rate, resolve_client_today,
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

/// The interest fields after a create (`current = None`) or edit: absent request fields keep the
/// account's values, and turning interest on starts the daily job's cursor at `as_of` (the
/// client's today), so no past day is credited.
fn resolve_interest(
    rate_bps: Option<i32>,
    rate_type: Option<&str>,
    current: Option<&AccountRow>,
    as_of: NaiveDate,
) -> Result<AccountInterest, ApiError> {
    let interest_rate_bps = match rate_bps {
        Some(bps) => require_interest_rate(bps)?,
        None => current.map_or(0, |account| account.interest_rate_bps),
    };
    let interest_rate_type = match rate_type {
        Some(value) => parse_interest_rate_type(value)?,
        None => current.map_or(InterestRateType::Apy, |account| account.interest_rate_type),
    };
    let was_earning = current.is_some_and(|account| account.interest_rate_bps > 0);
    let interest_accrued_through = if interest_rate_bps > 0 && !was_earning {
        Some(as_of)
    } else {
        current.and_then(|account| account.interest_accrued_through)
    };
    Ok(AccountInterest {
        interest_rate_bps,
        interest_rate_type,
        interest_accrued_through,
    })
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
    Query(query): Query<AsOfQuery>,
    Json(body): Json<CreateAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let as_of = resolve_client_today(query.as_of.as_deref())?;
    let name = parse_optional_name(body.name.as_deref())?;
    let currency = parse_currency(&body.currency)?;
    let initial_amount = require_initial_amount(body.initial_amount)?;
    let interest = resolve_interest(
        Some(body.interest_rate_bps),
        body.interest_rate_type.as_deref(),
        None,
        as_of,
    )?;

    let row = accounts_repo::create(
        &state.db_pool,
        user.sub,
        name.as_deref(),
        currency,
        initial_amount,
        body.exclude_from_projections,
        &interest,
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
    // field is ignored. Only name, initial amount, the projection flag and interest are updated.
    let name = parse_optional_name(body.name.as_deref())?;
    let initial_amount = require_initial_amount(body.initial_amount)?;
    let existing = accounts_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let interest = resolve_interest(
        body.interest_rate_bps,
        body.interest_rate_type.as_deref(),
        Some(&existing),
        as_of,
    )?;

    accounts_repo::update(
        &state.db_pool,
        user.sub,
        id,
        name.as_deref(),
        initial_amount,
        body.exclude_from_projections,
        &interest,
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

/// The account's daily interest credits, newest first. They stay out of `GET /income` so they
/// don't crowd it; clients list them per account instead.
pub async fn list_account_interest(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<InterestCreditResponse>>, ApiError> {
    let credits = income_repo::list_interest(&state.db_pool, user.sub, id).await?;
    Ok(Json(
        credits
            .into_iter()
            .map(|(date, amount)| InterestCreditResponse { date, amount })
            .collect(),
    ))
}
