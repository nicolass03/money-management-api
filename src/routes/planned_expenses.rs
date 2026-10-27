use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{
    CreatePlannedExpenseRequest, PayPlannedExpenseRequest, UpdatePlannedExpenseRequest,
};
use crate::error::ApiError;
use crate::models::{expense_to_response, planned_to_response, ExpenseResponse, PlannedExpenseResponse};
use crate::repos::{expenses as expenses_repo, planned_expenses as planned_repo};
use crate::routes::helpers::{
    finish_write, pick_payment_account, resolve_account, resolve_account_for_update,
};
use crate::services::materialization::build_planned_materialized_set;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_tag_names, require_non_empty_name, require_positive_amount,
    resolve_client_today,
};

type ValidatedPlanned = (
    String,
    Option<chrono::NaiveDate>,
    i32,
    crate::models::CurrencyCode,
    Vec<String>,
);

/// A missing or blank `date` means undated (e.g. a debt with no due date). With `after` (the
/// client's today), a dated item must fall after it.
fn validate_planned(
    name: &str,
    date: Option<&str>,
    amount: i32,
    currency: &str,
    tags: &[String],
    after: Option<chrono::NaiveDate>,
) -> Result<ValidatedPlanned, ApiError> {
    let name = require_non_empty_name(name)?;
    let tags = parse_tag_names(tags)?;
    let date = date
        .map(str::trim)
        .filter(|date| !date.is_empty())
        .map(parse_date)
        .transpose()?;
    if let (Some(date), Some(today)) = (date, after) {
        if date <= today {
            return Err(ApiError::BadRequest("date must be in the future".into()));
        }
    }
    let amount = require_positive_amount(amount)?;
    let currency = parse_currency(currency)?;
    Ok((name, date, amount, currency, tags))
}

pub async fn list_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<PlannedExpenseResponse>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;
    let revision = settings.cache_revision;
    let rows = state.loader.planned_with_tags(user.sub, revision).await?;
    let expenses = state.loader.expenses_with_tags(user.sub, revision).await?;
    let paid = build_planned_materialized_set(expenses.iter().map(|(row, _)| row));
    Ok(Json(
        rows.iter()
            .map(|(row, tags)| planned_to_response(row.clone(), tags.clone(), paid.contains(&row.id)))
            .collect(),
    ))
}

async fn is_paid(state: &AppState, user_id: Uuid, id: Uuid) -> Result<bool, ApiError> {
    Ok(expenses_repo::find_by_planned_id(&state.db_pool, user_id, id)
        .await?
        .is_some())
}

pub async fn create_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreatePlannedExpenseRequest>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (name, date, amount, currency, tags) = validate_planned(
        &body.name,
        body.date.as_deref(),
        body.amount,
        &body.currency,
        &body.tags,
        Some(resolve_client_today(body.as_of.as_deref())?),
    )?;
    let (account_id, currency) =
        resolve_account(&state.db_pool, user.sub, body.account_id, currency).await?;
    let row = planned_repo::create(
        &state.db_pool,
        user.sub,
        &name,
        date,
        amount,
        currency,
        &tags,
        account_id,
    )
    .await?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(planned_to_response(row, tags, false)))
}

pub async fn get_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (row, tags) = planned_repo::find_with_tags(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let paid = is_paid(&state, user.sub, id).await?;
    Ok(Json(planned_to_response(row, tags, paid)))
}

pub async fn update_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePlannedExpenseRequest>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (name, date, amount, currency, tags) = validate_planned(
        &body.name,
        body.date.as_deref(),
        body.amount,
        &body.currency,
        &body.tags,
        None,
    )?;
    let existing = planned_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (account_id, currency) = resolve_account_for_update(
        &state.db_pool,
        user.sub,
        body.account_id,
        existing.account_id,
        currency,
    )
    .await?;
    let row = planned_repo::update(
        &state.db_pool,
        user.sub,
        id,
        &name,
        date,
        amount,
        currency,
        &tags,
        account_id,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    finish_write(&state, user.sub, None).await;
    let paid = is_paid(&state, user.sub, id).await?;
    Ok(Json(planned_to_response(row, tags, paid)))
}

pub async fn delete_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !planned_repo::soft_delete(&state.db_pool, user.sub, id).await? {
        return Err(ApiError::NotFound);
    }
    finish_write(&state, user.sub, None).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Records a one-time expense (dated or undated) as paid today (the client's `asOf`). Full payment
/// only: an item is paid once, though the amount may differ from the planned one. Draws from the
/// item's account while it is active, else a same-currency account. The expense links back via
/// `planned_expense_id`, so the item shows as paid and stays out of projections.
pub async fn pay_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PayPlannedExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let amount = require_positive_amount(body.amount)?;
    let planned = planned_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if is_paid(&state, user.sub, id).await? {
        return Err(ApiError::BadRequest(
            "this payment has already been recorded".into(),
        ));
    }
    let today = resolve_client_today(body.as_of.as_deref())?;
    let account_id = pick_payment_account(
        &state.db_pool,
        user.sub,
        planned.account_id,
        planned.currency,
        amount,
        today,
    )
    .await?;
    let row = expenses_repo::create_early_paid(
        &state.db_pool,
        user.sub,
        &planned.name,
        amount,
        planned.currency,
        today,
        planned.date,
        None,
        Some(planned.id),
        account_id,
        amount != planned.amount,
        false,
    )
    .await?;
    let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, row.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(expense_to_response(row, tags)))
}
