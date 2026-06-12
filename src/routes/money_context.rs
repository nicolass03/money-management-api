use axum::extract::{Query, State};
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::{MoneyContextQuery, MoneyContextResponse};
use crate::error::ApiError;
use crate::repos::settings as settings_repo;
use crate::services::exchange_rates::get_exchange_rates;
use crate::state::AppState;

pub async fn get_money_context(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<MoneyContextQuery>,
) -> Result<Json<MoneyContextResponse>, ApiError> {
    if query.force_refresh && state.rate_limit_enabled {
        state
            .force_refresh_limiter
            .check_key(&user.sub)
            .map_err(|_| ApiError::BadRequest("rate limit exceeded".into()))?;
    }

    let settings = settings_repo::get_user_settings(&state.db_pool, user.sub).await?;
    let rates = get_exchange_rates(&state.db_pool, query.force_refresh).await?;
    Ok(Json(MoneyContextResponse {
        display_currency: settings.display_currency,
        rates,
    }))
}
