use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::ReportSummaryQuery;
use crate::error::ApiError;
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::reports::{build_report_summary, validate_report_range, ReportSummaryResponse};
use crate::state::AppState;

/// Reports over actual (persisted) expenses and income. Built from the cached full lists — the
/// report's own range and its prior period are filtered in memory — so a warm report costs no
/// database round trip.
pub async fn get_report_summary(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ReportSummaryQuery>,
) -> Result<Json<ReportSummaryResponse>, ApiError> {
    let (from, to, _) =
        validate_report_range(&query.from, &query.to).map_err(ApiError::BadRequest)?;

    let settings = state.loader.user_settings(user.sub).await?;
    let revision = settings.cache_revision;
    let rates = get_exchange_rates(&state.db_pool, false).await?;
    let expenses = state.loader.expenses_with_tags(user.sub, revision).await?;
    let income = state.loader.income_list(user.sub, revision).await?;
    let primary_schedule = state.loader.primary_schedule(user.sub, &settings).await?;

    Ok(Json(build_report_summary(
        from,
        to,
        &expenses,
        &income,
        primary_schedule.as_ref(),
        settings.display_currency,
        rates,
        query.compare_prior,
    )))
}
