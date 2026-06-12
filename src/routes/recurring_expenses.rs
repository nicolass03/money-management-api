use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{CreateRecurringExpenseRequest, UpdateRecurringExpenseRequest};
use crate::error::ApiError;
use crate::models::{recurring_to_response, RecurringExpenseResponse};
use crate::repos::recurring_expenses as recurring_repo;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_pay_frequency, parse_tag_names, require_non_empty_name,
    require_positive_amount,
};

fn validate_recurring(
    body: &CreateRecurringExpenseRequest,
) -> Result<
    (
        String,
        chrono::NaiveDate,
        crate::models::PayFrequency,
        i32,
        crate::models::CurrencyCode,
        Vec<String>,
        bool,
        Option<chrono::NaiveDate>,
    ),
    ApiError,
> {
    let name = require_non_empty_name(&body.name)?;
    let tags = parse_tag_names(&body.tags)?;
    let anchor_date = parse_date(&body.anchor_date)
        .map_err(|_| ApiError::BadRequest("invalid anchor date".into()))?;
    let frequency = parse_pay_frequency(&body.frequency)?;
    let amount = require_positive_amount(body.amount)?;
    let currency = parse_currency(&body.currency)?;
    let last_payment_date = match &body.last_payment_date {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => {
            let date = parse_date(value)
                .map_err(|_| ApiError::BadRequest("invalid last payment date".into()))?;
            let anchor_s = anchor_date.format("%Y-%m-%d").to_string();
            if value < &anchor_s {
                return Err(ApiError::BadRequest(
                    "last payment date must be on or after anchor date".into(),
                ));
            }
            Some(date)
        }
        None => None,
    };
    Ok((
        name,
        anchor_date,
        frequency,
        amount,
        currency,
        tags,
        body.is_subscription,
        last_payment_date,
    ))
}

pub async fn list_recurring(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<RecurringExpenseResponse>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;
    let rows = state
        .loader
        .recurring_with_tags(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(row, tags)| recurring_to_response(row, tags))
            .collect(),
    ))
}

pub async fn create_recurring(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateRecurringExpenseRequest>,
) -> Result<Json<RecurringExpenseResponse>, ApiError> {
    let (name, anchor_date, frequency, amount, currency, tags, is_subscription, last_payment_date) =
        validate_recurring(&body)?;
    let row = recurring_repo::create(
        &state.db_pool,
        user.sub,
        &name,
        anchor_date,
        frequency,
        amount,
        currency,
        &tags,
        is_subscription,
        last_payment_date,
    )
    .await?;
    state
        .cache
        .invalidate(InvalidationScope::RecurringChange, user.sub);
    Ok(Json(recurring_to_response(row, tags)))
}

pub async fn get_recurring(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<RecurringExpenseResponse>, ApiError> {
    let (row, tags) = recurring_repo::find_with_tags(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(recurring_to_response(row, tags)))
}

pub async fn update_recurring(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRecurringExpenseRequest>,
) -> Result<Json<RecurringExpenseResponse>, ApiError> {
    let req = CreateRecurringExpenseRequest {
        name: body.name,
        anchor_date: body.anchor_date,
        frequency: body.frequency,
        amount: body.amount,
        currency: body.currency,
        tags: body.tags,
        is_subscription: body.is_subscription,
        last_payment_date: body.last_payment_date,
    };
    let (name, anchor_date, frequency, amount, currency, tags, is_subscription, last_payment_date) =
        validate_recurring(&req)?;
    let row = recurring_repo::update(
        &state.db_pool,
        user.sub,
        id,
        &name,
        anchor_date,
        frequency,
        amount,
        currency,
        &tags,
        is_subscription,
        last_payment_date,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::RecurringChange, user.sub);
    Ok(Json(recurring_to_response(row, tags)))
}

pub async fn delete_recurring(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    recurring_repo::delete(&state.db_pool, user.sub, id).await?;
    state
        .cache
        .invalidate(InvalidationScope::RecurringChange, user.sub);
    Ok(Json(serde_json::json!({ "success": true })))
}
