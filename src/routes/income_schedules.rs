use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{CreateIncomeScheduleRequest, UpdateIncomeScheduleRequest};
use crate::error::ApiError;
use crate::models::IncomePayScheduleResponse;
use crate::repos::income_schedules as schedules_repo;
use crate::routes::helpers::resolve_account;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_pay_frequency, require_non_empty_name, require_positive_amount,
};

fn validate_schedule(
    name: &str,
    anchor_date: &str,
    frequency: &str,
    amount: i32,
    currency: &str,
) -> Result<(String, chrono::NaiveDate, crate::models::PayFrequency, i32, crate::models::CurrencyCode), ApiError>
{
    let name = require_non_empty_name(name)?;
    let anchor_date = parse_date(anchor_date).map_err(|_| ApiError::BadRequest("invalid anchor date".into()))?;
    let frequency = parse_pay_frequency(frequency)?;
    let amount = require_positive_amount(amount)?;
    let currency = parse_currency(currency)?;
    Ok((name, anchor_date, frequency, amount, currency))
}

pub async fn list_schedules(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<IncomePayScheduleResponse>>, ApiError> {
    let settings = state.loader.user_settings(user.sub).await?;
    let rows = state
        .loader
        .schedules_list(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

pub async fn create_schedule(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateIncomeScheduleRequest>,
) -> Result<Json<IncomePayScheduleResponse>, ApiError> {
    let (name, anchor_date, frequency, amount, currency) = validate_schedule(
        &body.name,
        &body.anchor_date,
        &body.frequency,
        body.amount,
        &body.currency,
    )?;
    let (account_id, currency) =
        resolve_account(&state.db_pool, user.sub, body.account_id, currency).await?;
    let schedule = schedules_repo::create(
        &state.db_pool,
        user.sub,
        &name,
        anchor_date,
        frequency,
        amount,
        currency,
        account_id,
    )
    .await?;
    state
        .cache
        .invalidate(InvalidationScope::ScheduleChange, user.sub).await;
    Ok(Json(schedule.into()))
}

pub async fn get_schedule(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<IncomePayScheduleResponse>, ApiError> {
    let schedule = schedules_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(schedule.into()))
}

pub async fn update_schedule(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateIncomeScheduleRequest>,
) -> Result<Json<IncomePayScheduleResponse>, ApiError> {
    let (name, anchor_date, frequency, amount, currency) = validate_schedule(
        &body.name,
        &body.anchor_date,
        &body.frequency,
        body.amount,
        &body.currency,
    )?;
    let (account_id, currency) =
        resolve_account(&state.db_pool, user.sub, body.account_id, currency).await?;
    let schedule = schedules_repo::update(
        &state.db_pool,
        user.sub,
        id,
        &name,
        anchor_date,
        frequency,
        amount,
        currency,
        account_id,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::ScheduleChange, user.sub).await;
    Ok(Json(schedule.into()))
}

pub async fn delete_schedule(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    schedules_repo::delete(&state.db_pool, user.sub, id).await?;
    state
        .cache
        .invalidate(InvalidationScope::ScheduleChange, user.sub).await;
    Ok(Json(serde_json::json!({ "success": true })))
}
