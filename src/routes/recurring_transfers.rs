use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::RecurringTransferRequest;
use crate::error::ApiError;
use crate::models::RecurringTransferRow;
use crate::repos::recurring_transfers::{self as recurring_repo, RecurringTransferFields};
use crate::routes::helpers::{finish_write, resolve_transfer_accounts};
use crate::state::AppState;
use crate::validation::{
    parse_currency, parse_date, parse_optional_name, parse_pay_frequency, require_positive_amount,
};

// Templates are materialized into transfers by the daily job (`charge_due_transfers`); like
// transfers they never affect projections, so writes finish without re-freezing history.

fn validate_recurring_transfer(
    body: &RecurringTransferRequest,
) -> Result<RecurringTransferFields, ApiError> {
    Ok(RecurringTransferFields {
        name: parse_optional_name(body.name.as_deref())?,
        from_account_id: body.from_account_id,
        to_account_id: body.to_account_id,
        amount: require_positive_amount(body.amount)?,
        currency: parse_currency(&body.currency)?,
        anchor_date: parse_date(&body.anchor_date)
            .map_err(|_| ApiError::BadRequest("invalid anchor date".into()))?,
        frequency: parse_pay_frequency(&body.frequency)?,
    })
}

pub async fn list_recurring_transfers(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<RecurringTransferRow>>, ApiError> {
    Ok(Json(recurring_repo::list_all(&state.db_pool, user.sub).await?))
}

pub async fn create_recurring_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<RecurringTransferRequest>,
) -> Result<Json<RecurringTransferRow>, ApiError> {
    let fields = validate_recurring_transfer(&body)?;
    resolve_transfer_accounts(
        &state.db_pool,
        user.sub,
        fields.from_account_id,
        fields.to_account_id,
        fields.currency,
        None,
    )
    .await?;
    let row = recurring_repo::create(&state.db_pool, user.sub, &fields).await?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(row))
}

pub async fn update_recurring_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RecurringTransferRequest>,
) -> Result<Json<RecurringTransferRow>, ApiError> {
    let existing = recurring_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let fields = validate_recurring_transfer(&body)?;
    resolve_transfer_accounts(
        &state.db_pool,
        user.sub,
        fields.from_account_id,
        fields.to_account_id,
        fields.currency,
        Some((existing.from_account_id, existing.to_account_id)),
    )
    .await?;
    let row = recurring_repo::update(&state.db_pool, user.sub, id, &fields)
        .await?
        .ok_or(ApiError::NotFound)?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(row))
}

pub async fn delete_recurring_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !recurring_repo::soft_delete(&state.db_pool, user.sub, id).await? {
        return Err(ApiError::NotFound);
    }
    finish_write(&state, user.sub, None).await;
    Ok(Json(serde_json::json!({ "success": true })))
}
