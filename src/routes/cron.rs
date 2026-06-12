use axum::extract::State;
use axum::Json;

use crate::dto::DailyExpensesResponse;
use crate::error::ApiError;
use crate::repos::users;
use crate::services::charge_due_expenses::charge_due_expenses_for_date;
use crate::state::AppState;
use crate::validation::today_iso;

pub async fn daily_expenses(
    State(state): State<AppState>,
) -> Result<Json<DailyExpensesResponse>, ApiError> {
    let date = today_iso();
    let user_ids = users::list_user_ids(&state.db_pool).await?;
    let mut created = 0;
    for user_id in user_ids {
        created += charge_due_expenses_for_date(&state.db_pool, user_id, &date).await?;
    }
    Ok(Json(DailyExpensesResponse { date, created }))
}
