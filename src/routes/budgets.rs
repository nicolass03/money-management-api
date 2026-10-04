use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{
    CompleteBudgetRequest, CreateBudgetExpenseRequest, CreateBudgetRequest, UpdateBudgetRequest,
};
use crate::error::ApiError;
use crate::models::{budget_to_response, expense_to_response, BudgetResponse, ExpenseResponse};
use crate::repos::{budgets as budgets_repo, expenses as expenses_repo};
use crate::routes::helpers::{finish_write, pick_payment_account};
use crate::services::projection_history::HistorySync;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_tag_names, regex_like_date, require_non_empty_name,
    require_positive_amount, today_iso,
};

fn parse_optional_date(value: &Option<String>) -> Result<Option<chrono::NaiveDate>, ApiError> {
    match value {
        None => Ok(None),
        Some(v) if v.trim().is_empty() => Ok(None),
        Some(v) => {
            if !regex_like_date(v) {
                return Err(ApiError::BadRequest("invalid date".into()));
            }
            Ok(Some(parse_date(v)?))
        }
    }
}

/// History refresh for a budget change touching these dates. Projections show a dated budget as a
/// single line on its end date (its expenses hidden), and an undated budget's expenses at their own
/// dates — so the earliest affected date is the earliest of the given end dates and, when the
/// change can flip between those two representations, the budget's earliest expense date.
fn budget_history(dates: impl IntoIterator<Item = Option<chrono::NaiveDate>>) -> Option<HistorySync> {
    dates.into_iter().flatten().min().map(HistorySync::From)
}

fn validate_budget(
    body: &CreateBudgetRequest,
) -> Result<
    (
        String,
        i32,
        crate::models::CurrencyCode,
        Option<chrono::NaiveDate>,
        Option<chrono::NaiveDate>,
        Vec<String>,
    ),
    ApiError,
> {
    let name = require_non_empty_name(&body.name)?;
    let tags = parse_tag_names(&body.tags)?;
    let amount = require_positive_amount(body.amount)?;
    let currency = parse_currency(&body.currency)?;
    let start_date = parse_optional_date(&body.start_date)?;
    let end_date = parse_optional_date(&body.end_date)?;
    let has_start = start_date.is_some();
    let has_end = end_date.is_some();
    if has_start != has_end {
        return Err(ApiError::BadRequest(
            "dated budgets require both start and end dates".into(),
        ));
    }
    if let (Some(start), Some(end)) = (start_date, end_date) {
        if end < start {
            return Err(ApiError::BadRequest(
                "end date must be on or after start date".into(),
            ));
        }
    }
    Ok((name, amount, currency, start_date, end_date, tags))
}

pub async fn list_budgets(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<BudgetResponse>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;
    let rows = state
        .loader
        .budgets_with_tags_and_spent(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(row, tags, spent)| budget_to_response(row, tags, spent))
            .collect(),
    ))
}

pub async fn create_budget(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateBudgetRequest>,
) -> Result<Json<BudgetResponse>, ApiError> {
    let (name, amount, currency, start_date, end_date, tags) = validate_budget(&body)?;
    let row = budgets_repo::create(
        &state.db_pool,
        user.sub,
        &name,
        amount,
        currency,
        start_date,
        end_date,
        &tags,
    )
    .await?;
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([row.end_date]),
    )
    .await;
    Ok(Json(budget_to_response(row, tags, 0)))
}

pub async fn get_budget(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BudgetResponse>, ApiError> {
    let (row, tags, spent) = budgets_repo::find_with_tags_and_spent(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(budget_to_response(row, tags, spent)))
}

pub async fn update_budget(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBudgetRequest>,
) -> Result<Json<BudgetResponse>, ApiError> {
    let existing = budgets_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if existing.completed_at.is_some() {
        return Err(ApiError::BadRequest("budget is completed".into()));
    }

    let req = CreateBudgetRequest {
        name: body.name,
        amount: body.amount,
        currency: body.currency,
        start_date: body.start_date,
        end_date: body.end_date,
        tags: body.tags,
    };
    let (name, amount, currency, start_date, end_date, tags) = validate_budget(&req)?;
    // `spent` is a raw sum of the budget's expenses, all stored in the budget's currency; changing
    // it afterwards would reinterpret those amounts in another currency.
    let earliest_expense =
        budgets_repo::earliest_expense_date_for(&state.db_pool, user.sub, id).await?;
    if currency != existing.currency && earliest_expense.is_some() {
        return Err(ApiError::BadRequest(
            "cannot change the currency of a budget with recorded expenses".into(),
        ));
    }
    let row = budgets_repo::update(
        &state.db_pool,
        user.sub,
        id,
        &name,
        amount,
        currency,
        start_date,
        end_date,
        &tags,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    let spent = budgets_repo::find_with_tags_and_spent(&state.db_pool, user.sub, id)
        .await?
        .map(|(_, _, spent)| spent)
        .unwrap_or(0);
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([existing.end_date, row.end_date, earliest_expense]),
    )
    .await;
    Ok(Json(budget_to_response(row, tags, spent)))
}

pub async fn delete_budget(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let existing = budgets_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Only budgets without expenses can be deleted, so just its own line moves.
    budgets_repo::delete(&state.db_pool, user.sub, id).await?;
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([existing.end_date]),
    )
    .await;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn complete_budget(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CompleteBudgetRequest>,
) -> Result<Json<BudgetResponse>, ApiError> {
    let as_of = match &body.as_of {
        Some(value) if !value.trim().is_empty() => {
            if !regex_like_date(value) {
                return Err(ApiError::BadRequest("invalid date".into()));
            }
            parse_date(value)?
        }
        _ => parse_date(&today_iso())?,
    };

    let existing = budgets_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let row = budgets_repo::complete(&state.db_pool, user.sub, id, as_of).await?;
    let (_, tags, spent) = budgets_repo::find_with_tags_and_spent(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Completing moves the line to the new end date and can turn an open-ended budget's individual
    // expenses (shown at their own dates) into that single line.
    let earliest_expense =
        budgets_repo::earliest_expense_date_for(&state.db_pool, user.sub, id).await?;
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([existing.end_date, row.end_date, earliest_expense]),
    )
    .await;
    Ok(Json(budget_to_response(row, tags, spent)))
}

pub async fn list_budget_expenses(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(budget_id): Path<Uuid>,
) -> Result<Json<Vec<ExpenseResponse>>, ApiError> {
    budgets_repo::find_by_id(&state.db_pool, user.sub, budget_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let rows = expenses_repo::list_by_budget(&state.db_pool, user.sub, budget_id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(row, tags)| expense_to_response(row, tags))
            .collect(),
    ))
}

pub async fn create_budget_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(budget_id): Path<Uuid>,
    Json(body): Json<CreateBudgetExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let budget = budgets_repo::find_with_tags_and_spent(&state.db_pool, user.sub, budget_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (budget_row, _budget_tags, _spent) = budget;
    if budget_row.completed_at.is_some() {
        return Err(ApiError::BadRequest("budget is completed".into()));
    }
    let amount = require_positive_amount(body.amount)?;
    let date = parse_date(&body.date)?;

    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&budget_row.name)
        .to_string();

    // Draw from an account in the budget's currency (one that covers it, else the richest), so the
    // spend shows in account balances. Same-currency only: `spent` sums raw amounts in the budget's
    // currency. Stays unassigned when the user holds no account in that currency.
    let account_id = pick_payment_account(
        &state.db_pool,
        user.sub,
        None,
        budget_row.currency,
        amount,
        date,
    )
    .await?;
    let row = budgets_repo::create_budget_expense(
        &state.db_pool,
        user.sub,
        budget_id,
        &name,
        amount,
        budget_row.currency,
        date,
        account_id,
    )
    .await?;
    let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, row.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([Some(date), budget_row.end_date]),
    )
    .await;
    Ok(Json(expense_to_response(row, tags)))
}

pub async fn delete_budget_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((budget_id, expense_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let budget = budgets_repo::find_by_id(&state.db_pool, user.sub, budget_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let deleted =
        budgets_repo::delete_budget_expense(&state.db_pool, user.sub, budget_id, expense_id)
            .await?
            .ok_or(ApiError::NotFound)?;
    finish_write(
        &state,
        user.sub,
        InvalidationScope::BudgetChange,
        budget_history([Some(deleted.date), budget.end_date]),
    )
    .await;
    Ok(Json(serde_json::json!({ "success": true })))
}
