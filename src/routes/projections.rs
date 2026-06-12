use axum::extract::State;
use axum::Json;

use crate::auth::extractor::AuthenticatedUser;
use crate::dto::ProjectionsResponse;
use crate::error::ApiError;
use crate::repos::{
    budgets, expenses, income, income_schedules, planned_expenses, recurring_expenses, settings,
};
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::projections::build_projection_rows;
use crate::state::AppState;
use crate::validation::today_iso;

pub async fn get_projections(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ProjectionsResponse>, ApiError> {
    let user_settings = settings::get_user_settings(&state.db_pool, user.sub).await?;
    let Some(schedule_id) = user_settings.primary_schedule_id else {
        return Err(ApiError::BadRequest(
            "set a primary pay schedule in settings first".into(),
        ));
    };
    let Some(primary_schedule) =
        income_schedules::find_by_id(&state.db_pool, user.sub, schedule_id).await?
    else {
        return Err(ApiError::BadRequest("primary schedule not found".into()));
    };

    let rates = get_exchange_rates(&state.db_pool, false).await?;
    let income_entries = income::list_all(&state.db_pool, user.sub).await?;
    let expenses = expenses::list_with_tags(&state.db_pool, user.sub).await?;
    let recurring = recurring_expenses::list_with_tags(&state.db_pool, user.sub).await?;
    let planned = planned_expenses::list_with_tags(&state.db_pool, user.sub).await?;
    let budgets = budgets::list_with_tags_and_spent(&state.db_pool, user.sub).await?;

    let projection_start_date = user_settings
        .projection_start_date
        .map(|d| d.format("%Y-%m-%d").to_string());
    let projection_start_ref = projection_start_date.as_deref();

    let rows = build_projection_rows(
        &primary_schedule,
        &income_entries,
        &expenses,
        &recurring,
        &planned,
        &budgets,
        user_settings.display_currency,
        &rates,
        user_settings.projection_initial_free_money,
        projection_start_ref,
        &today_iso(),
    );

    Ok(Json(ProjectionsResponse {
        rows,
        primary_schedule: primary_schedule.into(),
        display_currency: user_settings.display_currency,
        rates,
    }))
}
