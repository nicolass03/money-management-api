use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{CreateIncomeRequest, UpdateIncomeRequest};
use crate::error::ApiError;
use crate::models::{IncomeResponse, IncomeSource, is_manual_income};
use crate::repos::income as income_repo;
use crate::services::sync_scheduled_income::sync_all_scheduled_income;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, require_non_empty_name, require_positive_amount,
};

fn validate_income(
    name: &str,
    amount: i32,
    currency: &str,
    date: &str,
) -> Result<(String, i32, crate::models::CurrencyCode, chrono::NaiveDate), ApiError> {
    let name = require_non_empty_name(name)?;
    let amount = require_positive_amount(amount)?;
    let currency = parse_currency(currency)?;
    let date = parse_date(date)?;
    Ok((name, amount, currency, date))
}

pub async fn list_income(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<IncomeResponse>>, ApiError> {
    let rows = income_repo::list_all(&state.db_pool, user.sub).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

pub async fn create_income(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreateIncomeRequest>,
) -> Result<Json<IncomeResponse>, ApiError> {
    let (name, amount, currency, date) =
        validate_income(&body.name, body.amount, &body.currency, &body.date)?;
    let row = income_repo::create(
        &state.db_pool,
        user.sub,
        &name,
        amount,
        currency,
        IncomeSource::Manual,
        date,
    )
    .await?;
    Ok(Json(row.into()))
}

pub async fn get_income(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<IncomeResponse>, ApiError> {
    let row = income_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

pub async fn update_income(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateIncomeRequest>,
) -> Result<Json<IncomeResponse>, ApiError> {
    let existing = income_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !is_manual_income(&existing) {
        return Err(ApiError::BadRequest(
            "scheduled income cannot be edited here".into(),
        ));
    }
    let (name, amount, currency, date) =
        validate_income(&body.name, body.amount, &body.currency, &body.date)?;
    let row = income_repo::update(
        &state.db_pool,
        user.sub,
        id,
        &name,
        amount,
        currency,
        IncomeSource::Manual,
        date,
    )
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

pub async fn delete_income(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let existing = income_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !is_manual_income(&existing) {
        return Err(ApiError::BadRequest(
            "scheduled income cannot be deleted here".into(),
        ));
    }
    income_repo::delete(&state.db_pool, user.sub, id).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn sync_scheduled(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    sync_all_scheduled_income(&state.db_pool, user.sub).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}
