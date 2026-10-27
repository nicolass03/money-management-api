use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::TransferRequest;
use crate::error::ApiError;
use crate::models::TransferRow;
use crate::repos::transfers::{self as transfers_repo, TransferFields};
use crate::routes::helpers::{finish_write, resolve_transfer_accounts};
use crate::state::AppState;
use crate::validation::{parse_currency, parse_date, parse_optional_name, require_positive_amount};

// Transfers only move money between the user's own same-currency accounts: they change account
// balances but are net-zero for projections, so writes finish without re-freezing history.

fn validate_transfer(body: &TransferRequest) -> Result<TransferFields, ApiError> {
    Ok(TransferFields {
        name: parse_optional_name(body.name.as_deref())?,
        from_account_id: body.from_account_id,
        to_account_id: body.to_account_id,
        amount: require_positive_amount(body.amount)?,
        currency: parse_currency(&body.currency)?,
        date: parse_date(&body.date)?,
    })
}

pub async fn list_transfers(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<TransferRow>>, ApiError> {
    Ok(Json(transfers_repo::list_all(&state.db_pool, user.sub).await?))
}

pub async fn create_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TransferRequest>,
) -> Result<Json<TransferRow>, ApiError> {
    let fields = validate_transfer(&body)?;
    resolve_transfer_accounts(
        &state.db_pool,
        user.sub,
        fields.from_account_id,
        fields.to_account_id,
        fields.currency,
        None,
    )
    .await?;
    let row = transfers_repo::create(&state.db_pool, user.sub, &fields).await?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(row))
}

pub async fn update_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferRequest>,
) -> Result<Json<TransferRow>, ApiError> {
    let existing = transfers_repo::find_by_id(&state.db_pool, user.sub, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let fields = validate_transfer(&body)?;
    // A recurring occurrence's date and accounts belong to its schedule: moving the date would free
    // its (recurring_transfer_id, date) slot and let the daily job's catch-up re-create it.
    if existing.recurring_transfer_id.is_some()
        && (fields.date != existing.date
            || fields.from_account_id != existing.from_account_id
            || fields.to_account_id != existing.to_account_id)
    {
        return Err(ApiError::BadRequest(
            "a recurring transfer's date and accounts follow its schedule".into(),
        ));
    }
    resolve_transfer_accounts(
        &state.db_pool,
        user.sub,
        fields.from_account_id,
        fields.to_account_id,
        fields.currency,
        Some((existing.from_account_id, existing.to_account_id)),
    )
    .await?;
    let row = transfers_repo::update(&state.db_pool, user.sub, id, &fields)
        .await?
        .ok_or(ApiError::NotFound)?;
    finish_write(&state, user.sub, None).await;
    Ok(Json(row))
}

pub async fn delete_transfer(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !transfers_repo::soft_delete(&state.db_pool, user.sub, id).await? {
        return Err(ApiError::NotFound);
    }
    finish_write(&state, user.sub, None).await;
    Ok(Json(serde_json::json!({ "success": true })))
}
