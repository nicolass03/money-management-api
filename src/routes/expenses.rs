use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{
    CreateExpenseRequest, EarlyPayExpenseRequest, ExpensePeriodViewQuery, ExpensesQuery,
    PatchExpenseRequest, UpcomingPayableQuery,
};
use crate::error::ApiError;
use crate::models::{expense_to_response, ExpenseResponse};
use crate::repos::{expenses as expenses_repo, planned_expenses, recurring_expenses};
use crate::routes::helpers::get_current_pay_period;
use crate::services::pay_periods::{get_pay_dates_in_range, is_date_in_period, schedule_from_recurring};
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_tag_names, require_non_empty_name, require_positive_amount,
    today_iso,
};

pub async fn get_expense_period_view(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ExpensePeriodViewQuery>,
) -> Result<Json<crate::services::expense_period::ExpensePeriodViewResponse>, ApiError> {
    let response = state
        .loader
        .expense_period_view(user.sub, &query.period, query.include_projected)
        .await?;
    Ok(Json(response))
}

pub async fn get_upcoming_payable(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<UpcomingPayableQuery>,
) -> Result<Json<Vec<crate::services::upcoming_payable::PayableFutureItem>>, ApiError> {
    let items = state
        .loader
        .upcoming_payable(user.sub, query.horizon_days)
        .await?;
    Ok(Json(items))
}

pub async fn list_expenses(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ExpensesQuery>,
) -> Result<Json<Vec<ExpenseResponse>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;

    let rows = match (&query.from, &query.to) {
        (Some(from), Some(to)) => {
            let from_date = parse_date(from)?;
            let to_date = parse_date(to)?;
            expenses_repo::list_with_tags_in_range(&state.db_pool, user.sub, from_date, to_date)
                .await?
        }
        (None, None) => {
            state
                .loader
                .expenses_with_tags(user.sub, settings.cache_revision)
                .await?
        }
        _ => {
            return Err(ApiError::BadRequest(
                "both from and to query params are required for date filtering".into(),
            ));
        }
    };

    Ok(Json(
        rows.into_iter()
            .map(|(row, tags)| expense_to_response(row, tags))
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
    let currency = parse_currency(&body.currency)?;
    let date = parse_date(&body.date)?;

    let period = get_current_pay_period(&state.db_pool, user.sub)
        .await?
        .ok_or_else(|| ApiError::BadRequest("set a primary pay schedule in settings first".into()))?;
    let date_s = date.format("%Y-%m-%d").to_string();
    if !is_date_in_period(&date_s, &period) {
        return Err(ApiError::BadRequest(
            "date must fall within the current pay period".into(),
        ));
    }

    let row = expenses_repo::create_manual(
        &state.db_pool,
        user.sub,
        &name,
        amount,
        currency,
        date,
        &tags,
        body.is_subscription,
    )
    .await?;
    state
        .cache
        .invalidate(InvalidationScope::ExpenseChange, user.sub);
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

pub async fn patch_expense(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchExpenseRequest>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    let amount = require_positive_amount(body.amount)?;
    let existing = expenses_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if existing.is_system_generated() {
        return Err(ApiError::BadRequest(
            "cannot modify system-generated expense".into(),
        ));
    }
    let row = expenses_repo::update_amount(&state.db_pool, user.sub, id, amount)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::ExpenseChange, user.sub);
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
    state
        .cache
        .invalidate(InvalidationScope::ExpenseChange, user.sub);
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

    let period = get_current_pay_period(&state.db_pool, user.sub)
        .await?
        .ok_or_else(|| ApiError::BadRequest("set a primary pay schedule in settings first".into()))?;
    let paid_date_s = paid_date.format("%Y-%m-%d").to_string();
    if !is_date_in_period(&paid_date_s, &period) {
        return Err(ApiError::BadRequest(
            "paid date must fall within the current pay period".into(),
        ));
    }
    let today = today_iso();
    if paid_date_s > today {
        return Err(ApiError::BadRequest("paid date cannot be in the future".into()));
    }

    let scheduled_date_s = scheduled_date.format("%Y-%m-%d").to_string();
    if scheduled_date_s <= today {
        return Err(ApiError::BadRequest("scheduled date must be in the future".into()));
    }

    let row = if body.source_type == "recurring" {
        let recurring_id = body.recurring_id.ok_or_else(|| {
            ApiError::BadRequest("invalid recurring expense".into())
        })?;
        let recurring = recurring_expenses::find_by_id(&state.db_pool, user.sub, recurring_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("recurring expense not found".into()))?;
        let due_dates = get_pay_dates_in_range(
            &schedule_from_recurring(&recurring),
            &scheduled_date_s,
            &scheduled_date_s,
        );
        if due_dates.is_empty() {
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
            scheduled_date,
            Some(recurring_id),
            None,
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
        let planned_date_s = planned.date.format("%Y-%m-%d").to_string();
        if planned_date_s != scheduled_date_s {
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
            scheduled_date,
            None,
            Some(planned_id),
            amount_overridden,
            false,
        )
        .await?
    };

    let (_, tags) = expenses_repo::find_with_tags(&state.db_pool, user.sub, row.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::ExpenseChange, user.sub);
    Ok(Json(expense_to_response(row, tags)))
}
