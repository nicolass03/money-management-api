use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{
    CreateExpenseRequest, EarlyPayExpenseRequest, ExpensePeriodViewQuery, ExpensesQuery,
    PatchExpenseRequest, UpcomingPayableQuery,
};
use crate::error::ApiError;
use crate::models::{expense_to_response, ExpenseResponse, ExpenseRow};
use crate::repos::expenses::{self as expenses_repo, ManualExpenseEdit};
use crate::repos::{planned_expenses, recurring_expenses};
use crate::routes::helpers::{
    expense_history_date, finish_write, get_current_pay_period, pick_payment_account,
    resolve_account, resolve_account_for_update,
};
use crate::services::expense_period::ExpensePeriodViewResponse;
use crate::services::pay_periods::{occurs_on, schedule_from_recurring};
use crate::services::projection_history::HistorySync;
use crate::services::upcoming_payable::PayableFutureItem;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_tag_names, require_non_empty_name,
    require_non_negative_amount, require_positive_amount, resolve_client_today,
};

pub async fn get_expense_period_view(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ExpensePeriodViewQuery>,
) -> Result<Json<Arc<ExpensePeriodViewResponse>>, ApiError> {
    let response = state
        .loader
        .expense_period_view(
            user.sub,
            &query.period,
            query.include_projected,
            query.as_of.as_deref(),
        )
        .await?;
    Ok(Json(response))
}

pub async fn get_upcoming_payable(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<UpcomingPayableQuery>,
) -> Result<Json<Arc<Vec<PayableFutureItem>>>, ApiError> {
    let items = state
        .loader
        .upcoming_payable(user.sub, query.horizon_days, query.as_of.as_deref())
        .await?;
    Ok(Json(items))
}

/// All expenses (newest first), or only those dated in `[from, to]` — filtered from the cached
/// list, so a range query costs no database round trip on a warm cache.
pub async fn list_expenses(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ExpensesQuery>,
) -> Result<Json<Vec<ExpenseResponse>>, ApiError> {
    let range = match (&query.from, &query.to) {
        (Some(from), Some(to)) => Some((parse_date(from)?, parse_date(to)?)),
        (None, None) => None,
        _ => {
            return Err(ApiError::BadRequest(
                "both from and to query params are required for date filtering".into(),
            ));
        }
    };

    let settings = state.loader.user_settings(user.sub).await?;
    let rows = state
        .loader
        .expenses_with_tags(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(
        rows.iter()
            .filter(|(row, _)| range.is_none_or(|(from, to)| from <= row.date && row.date <= to))
            .map(|(row, tags)| expense_to_response(row.clone(), tags.clone()))
            .collect(),
    ))
}

pub async fn create_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let name = require_non_empty_name(&body.name)?;
    let tags = parse_tag_names(&body.tags)?;
    let amount = require_positive_amount(body.amount)?;
    let submitted_currency = parse_currency(&body.currency)?;
    let date = parse_date(&body.date)?;
    // Currency follows the selected account (if any), keeping derived balances single-currency.
    let (account_id, currency) =
        resolve_account(&state.db_pool, user.sub, body.account_id, submitted_currency).await?;

    let row = expenses_repo::create_manual(
        &state.db_pool,
        user.sub,
        &name,
        amount,
        currency,
        date,
        &tags,
        body.is_subscription,
        account_id,
    )
    .await?;
    // A past-dated expense must be reflected in the frozen projection history aggregates.
    finish_write(&state, user.sub, Some(HistorySync::From(row.date))).await;
    Ok(Json(expense_to_response(row, tags)))
}

pub async fn get_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let (row, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(expense_to_response(row, tags)))
}

fn is_manual(expense: &ExpenseRow) -> bool {
    expense.recurring_id.is_none()
        && expense.planned_expense_id.is_none()
        && expense.budget_id.is_none()
}

/// Edits an expense. With only `amount`, overrides the amount of any expense; a recurring charge
/// may be set to 0 to void an occurrence that didn't happen (the row keeps its slot, so the daily
/// job never re-charges it). With the full field set, edits a manual expense entirely.
pub async fn patch_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let existing = expenses_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let full_edit = body.name.is_some()
        || body.currency.is_some()
        || body.date.is_some()
        || body.tags.is_some()
        || body.is_subscription.is_some();

    let (row, tags) = if full_edit {
        if !is_manual(&existing) {
            return Err(ApiError::BadRequest(
                "only the amount of a recurring, planned or budget expense can be edited".into(),
            ));
        }
        let (Some(name), Some(currency), Some(date), Some(tags), Some(is_subscription)) =
            (&body.name, &body.currency, &body.date, &body.tags, body.is_subscription)
        else {
            return Err(ApiError::BadRequest(
                "a full edit needs name, currency, date, tags and isSubscription".into(),
            ));
        };
        let name = require_non_empty_name(name)?;
        let tags = parse_tag_names(tags)?;
        let amount = require_positive_amount(body.amount)?;
        let date = parse_date(date)?;
        let (account_id, currency) = resolve_account_for_update(
            &state.db_pool,
            user.sub,
            body.account_id,
            existing.account_id,
            parse_currency(currency)?,
        )
        .await?;
        let edit = ManualExpenseEdit {
            name: &name,
            amount,
            currency,
            date,
            tags: &tags,
            is_subscription,
            account_id,
        };
        let row = expenses_repo::update_manual(&state.db_pool, user.sub, id, edit)
            .await?
            .ok_or(ApiError::NotFound)?;
        (row, tags)
    } else {
        let amount = if existing.recurring_id.is_some() {
            require_non_negative_amount(body.amount)?
        } else {
            require_positive_amount(body.amount)?
        };
        let row = expenses_repo::update_amount(&state.db_pool, user.sub, id, amount)
            .await?
            .ok_or(ApiError::NotFound)?;
        let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, id)
            .await?
            .ok_or(ApiError::NotFound)?;
        (row, tags)
    };

    // The date may have moved (full edit): re-freezing from the earlier of the old/new dates
    // recomputes every period after it, so one trigger covers both.
    let history_date = expense_history_date(&state, user.sub, &existing)
        .await
        .min(row.date);
    finish_write(&state, user.sub, Some(HistorySync::From(history_date))).await;
    Ok(Json(expense_to_response(row, tags)))
}

pub async fn delete_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let existing = expenses_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if existing.is_system_generated() {
        return Err(ApiError::BadRequest(
            "cannot delete system-generated expense".into(),
        ));
    }
    expenses_repo::delete(&state.db_pool, user.sub, id).await?;
    let history_date = expense_history_date(&state, user.sub, &existing).await;
    finish_write(&state, user.sub, Some(HistorySync::From(history_date))).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn early_pay_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<EarlyPayExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    if body.source_type != "recurring" && body.source_type != "planned" {
        return Err(ApiError::BadRequest("invalid payment source".into()));
    }
    let scheduled_date = parse_date(&body.scheduled_date)
        .map_err(|_| ApiError::BadRequest("invalid scheduled date".into()))?;
    let paid_date = parse_date(&body.paid_date)
        .map_err(|_| ApiError::BadRequest("invalid paid date".into()))?;
    let currency = parse_currency(&body.currency)?;
    let amount = require_positive_amount(body.amount)?;
    let today = resolve_client_today(body.as_of.as_deref())?;

    let period = get_current_pay_period(&state, user.sub, today)
        .await?
        .ok_or_else(|| ApiError::BadRequest("set a primary pay schedule in settings first".into()))?;
    if !period.contains(paid_date) {
        return Err(ApiError::BadRequest(
            "paid date must fall within the current pay period".into(),
        ));
    }
    if paid_date > today {
        return Err(ApiError::BadRequest("paid date cannot be in the future".into()));
    }
    if scheduled_date <= today {
        return Err(ApiError::BadRequest("scheduled date must be in the future".into()));
    }

    // Draw the early payment from a funded account in its currency (else the richest such
    // account, which may go negative; else leave unassigned). Keeps currency-follows-account.
    let account_id =
        pick_payment_account(&state.db_pool, user.sub, None, currency, amount, paid_date).await?;

    let row = if body.source_type == "recurring" {
        let recurring_id = body.recurring_id.ok_or_else(|| {
            ApiError::BadRequest("invalid recurring expense".into())
        })?;
        let recurring = recurring_expenses::find_by_id(&state.db_pool, user.sub, recurring_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("recurring expense not found".into()))?;
        if !occurs_on(&schedule_from_recurring(&recurring), scheduled_date) {
            return Err(ApiError::BadRequest(
                "scheduled date does not match recurring expense".into(),
            ));
        }
        if expenses_repo::find_by_recurring_and_due_date(
            &state.db_pool,
            user.sub,
            recurring_id,
            scheduled_date,
        )
        .await?
        .is_some()
        {
            return Err(ApiError::BadRequest(
                "this payment has already been recorded".into(),
            ));
        }
        let amount_overridden =
            amount != recurring.amount || currency != recurring.currency;
        expenses_repo::create_early_paid(
            &state.db_pool,
            user.sub,
            &recurring.name,
            amount,
            currency,
            paid_date,
            Some(scheduled_date),
            Some(recurring_id),
            None,
            account_id,
            amount_overridden,
            recurring.is_subscription,
        )
        .await?
    } else {
        let planned_id = body.planned_expense_id.ok_or_else(|| {
            ApiError::BadRequest("invalid planned expense".into())
        })?;
        let planned = planned_expenses::find_by_id(&state.db_pool, user.sub, planned_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("planned expense not found".into()))?;
        if planned.date != Some(scheduled_date) {
            return Err(ApiError::BadRequest(
                "scheduled date does not match planned expense".into(),
            ));
        }
        if expenses_repo::find_by_planned_id(&state.db_pool, user.sub, planned_id)
            .await?
            .is_some()
        {
            return Err(ApiError::BadRequest(
                "this payment has already been recorded".into(),
            ));
        }
        let amount_overridden = amount != planned.amount || currency != planned.currency;
        expenses_repo::create_early_paid(
            &state.db_pool,
            user.sub,
            &planned.name,
            amount,
            currency,
            paid_date,
            Some(scheduled_date),
            None,
            Some(planned_id),
            account_id,
            amount_overridden,
            false,
        )
        .await?
    };

    let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, row.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(expense_to_response(row, tags)))
}
