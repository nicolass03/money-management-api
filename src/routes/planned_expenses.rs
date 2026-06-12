use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::cache::InvalidationScope;
use crate::dto::{CreatePlannedExpenseRequest, UpdatePlannedExpenseRequest};
use crate::error::ApiError;
use crate::models::{planned_to_response, PlannedExpenseResponse};
use crate::repos::planned_expenses as planned_repo;
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_tag_names, require_non_empty_name, require_positive_amount,
    today_iso,
};

fn validate_planned(
    name: &str,
    date: &str,
    amount: i32,
    currency: &str,
    tags: &[String],
    require_future: bool,
) -> Result<(String, chrono::NaiveDate, i32, crate::models::CurrencyCode, Vec<String>), ApiError> {
    let name = require_non_empty_name(name)?;
    let tags = parse_tag_names(tags)?;
    let date = parse_date(date)?;
    if require_future && date.format("%Y-%m-%d").to_string() <= today_iso() {
        return Err(ApiError::BadRequest("date must be in the future".into()));
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
    let rows = state
        .loader
        .planned_with_tags(user.sub, settings.cache_revision)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(row, tags)| planned_to_response(row, tags))
            .collect(),
    ))
}

pub async fn create_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<CreatePlannedExpenseRequest>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (name, date, amount, currency, tags) = validate_planned(
        &body.name,
        &body.date,
        body.amount,
        &body.currency,
        &body.tags,
        true,
    )?;
    let row = planned_repo::create(&state.db_pool, user.sub, &name, date, amount, currency, &tags).await?;
    state
        .cache
        .invalidate(InvalidationScope::PlannedChange, user.sub);
    Ok(Json(planned_to_response(row, tags)))
}

pub async fn get_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (row, tags) = planned_repo::find_with_tags(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(planned_to_response(row, tags)))
}

pub async fn update_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePlannedExpenseRequest>,
) -> Result<Json<PlannedExpenseResponse>, ApiError> {
    let (name, date, amount, currency, tags) = validate_planned(
        &body.name,
        &body.date,
        body.amount,
        &body.currency,
        &body.tags,
        false,
    )?;
    let row = planned_repo::update(&state.db_pool, user.sub, id, &name, date, amount, currency, &tags)
        .await?
        .ok_or(ApiError::NotFound)?;
    state
        .cache
        .invalidate(InvalidationScope::PlannedChange, user.sub);
    Ok(Json(planned_to_response(row, tags)))
}

pub async fn delete_planned(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    planned_repo::delete(&state.db_pool, user.sub, id).await?;
    state
        .cache
        .invalidate(InvalidationScope::PlannedChange, user.sub);
    Ok(Json(serde_json::json!({ "success": true })))
}
