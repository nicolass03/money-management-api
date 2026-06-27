use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::ReportSummaryQuery;
use crate::error::ApiError;
use crate::repos::{budgets as budgets_repo, expenses as expenses_repo, income as income_repo};
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::pay_periods::compare_iso;
use crate::services::reports::{build_report_summary, prior_period_range, validate_report_range};
use crate::state::AppState;
use crate::validation::parse_date;

pub async fn get_report_summary(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ReportSummaryQuery>,
) -> Result<Json<crate::services::reports::ReportSummaryResponse>, ApiError> {
    let (_, _, day_count) = validate_report_range(&query.from, &query.to)
        .map_err(ApiError::BadRequest)?;

    let from_date = parse_date(&query.from)?;
    let to_date = parse_date(&query.to)?;

    let load_from = if query.compare_prior {
        let (prior_from, _) = prior_period_range(&query.from, &query.to, day_count);
        if compare_iso(&prior_from, &query.from) < 0 {
            parse_date(&prior_from)?
        } else {
            from_date
        }
    } else {
        from_date
    };

    let settings = state.loader.user_settings(user.sub).await?;
    let rates = get_exchange_rates(&state.db_pool, false).await?;

    let expenses =
        expenses_repo::list_with_tags_in_range(&state.db_pool, user.sub, load_from, to_date)
            .await?;

    let income =
        income_repo::list_in_range(&state.db_pool, user.sub, load_from, to_date).await?;

    let budgets = budgets_repo::list_all(&state.db_pool, user.sub).await?;

    let response = build_report_summary(
        &query.from,
        &query.to,
        &expenses,
        &income,
        &budgets,
        settings.display_currency,
        rates,
        query.compare_prior,
    )
    .map_err(ApiError::BadRequest)?;

    Ok(Json(response))
}
