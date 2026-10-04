use std::sync::Arc;

use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::error::ApiError;
use crate::models::{
    IncomePayScheduleResponse, ProjectionHistoryRow, UserSettingsRow,
};
use crate::repos::{
    budgets, connection, expenses, income, income_schedules, planned_expenses,
    projection_history, recurring_expenses, settings, tags,
};
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::expense_period::{
    build_expense_period_view, ExpensePeriodKey, ExpensePeriodViewResponse,
};
use crate::services::pay_periods::{get_next_pay_date, get_period_for_pay_date, schedule_from_income};
use crate::services::projection_history::{build_rows_after, is_history_usable, load_projection_inputs};
use crate::services::projections::{build_projection_rows, ProjectionInputs, ProjectionRow};
use crate::services::upcoming_payable::{build_upcoming_payable_items, PayableFutureItem};
use crate::state::DbPool;
use crate::validation::resolve_reference_date;

use super::user_data_cache::UserDataCache;

#[derive(Clone)]
pub struct UserDataLoader {
    pool: DbPool,
    cache: Arc<UserDataCache>,
}

impl UserDataLoader {
    pub fn new(pool: DbPool, cache: Arc<UserDataCache>) -> Self {
        Self { pool, cache }
    }

    /// Returns the user's settings row — carrying the current `cache_revision` that keys
    /// every other cache — served from memory on a warm path so cache-backed reads need
    /// **zero** database round-trips. Eviction happens on every mutation (see
    /// `UserDataCache::invalidate`), with a short TTL as a safety net.
    async fn current_settings(&self, user_id: Uuid) -> Result<Arc<UserSettingsRow>, ApiError> {
        if let Some(cached) = self.cache.get_settings(user_id).await {
            return Ok(cached);
        }
        let row = Arc::new(settings::get_user_settings(&self.pool, user_id).await?);
        self.cache.set_settings(user_id, row.clone()).await;
        Ok(row)
    }

    pub async fn user_settings(&self, user_id: Uuid) -> Result<UserSettingsRow, ApiError> {
        Ok((*self.current_settings(user_id).await?).clone())
    }

    pub async fn expenses_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<super::user_data_cache::ExpensesWithTags, ApiError> {
        if let Some(cached) = self.cache.get_expenses(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = expenses::list_with_tags(&self.pool, user_id).await?;
        self.cache.set_expenses(user_id, revision, data.clone()).await;
        Ok(data)
    }

    pub async fn recurring_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<super::user_data_cache::RecurringWithTags, ApiError> {
        if let Some(cached) = self.cache.get_recurring(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = recurring_expenses::list_with_tags(&self.pool, user_id).await?;
        self.cache
            .set_recurring(user_id, revision, data.clone())
            .await;
        Ok(data)
    }

    pub async fn planned_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<super::user_data_cache::PlannedWithTags, ApiError> {
        if let Some(cached) = self.cache.get_planned(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = planned_expenses::list_with_tags(&self.pool, user_id).await?;
        self.cache.set_planned(user_id, revision, data.clone()).await;
        Ok(data)
    }

    pub async fn budgets_with_tags_and_spent(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<super::user_data_cache::BudgetsWithTagsAndSpent, ApiError> {
        if let Some(cached) = self.cache.get_budgets(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = budgets::list_with_tags_and_spent(&self.pool, user_id).await?;
        self.cache.set_budgets(user_id, revision, data.clone()).await;
        Ok(data)
    }

    pub async fn income_list(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Vec<crate::models::IncomeRow>, ApiError> {
        if let Some(cached) = self.cache.get_income(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = income::list_all(&self.pool, user_id).await?;
        self.cache.set_income(user_id, revision, data.clone()).await;
        Ok(data)
    }

    pub async fn schedules_list(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Vec<crate::models::IncomePayScheduleRow>, ApiError> {
        if let Some(cached) = self.cache.get_schedules(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let data = income_schedules::list_all(&self.pool, user_id).await?;
        self.cache
            .set_schedules(user_id, revision, data.clone())
            .await;
        Ok(data)
    }

    pub async fn tag_names(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Vec<String>, ApiError> {
        if let Some(cached) = self.cache.get_tags(user_id, revision).await {
            return Ok((*cached).clone());
        }
        let mut conn = connection::user_connection(&self.pool, user_id).await?;
        let data = tags::list_all_names(&mut conn, user_id).await?;
        self.cache.set_tags(user_id, revision, data.clone()).await;
        Ok(data)
    }

    pub async fn money_context(
        &self,
        user_id: Uuid,
        revision: i64,
        force_refresh: bool,
    ) -> Result<Arc<MoneyContextResponse>, ApiError> {
        if !force_refresh {
            if let Some(cached) = self.cache.get_money_context(user_id, revision).await {
                return Ok(cached);
            }
        }
        let user_settings = self.current_settings(user_id).await?;
        let rates = get_exchange_rates(&self.pool, force_refresh).await?;
        let response = Arc::new(MoneyContextResponse {
            display_currency: user_settings.display_currency,
            rates,
        });
        if !force_refresh {
            self.cache
                .set_money_context(user_id, revision, response.clone())
                .await;
        }
        Ok(response)
    }

    pub async fn projections(
        &self,
        user_id: Uuid,
        include_past: bool,
        as_of: Option<&str>,
    ) -> Result<Arc<ProjectionsResponse>, ApiError> {
        let user_settings = self.current_settings(user_id).await?;
        let revision = user_settings.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;

        if let Some(cached) = self
            .cache
            .get_projections(user_id, revision, &reference_date)
            .await
        {
            return Ok(filter_projection_rows(cached, include_past));
        }

        let inputs = self.projection_inputs(user_id).await?;

        // Past periods are served from the frozen history table; only the periods after the last
        // frozen one are computed live, seeded from its cumulative (which also self-heals any
        // closed period the daily job hasn't frozen yet — it re-appears as a live past row). With
        // nothing frozen (or rows pending a display-currency rebuild) everything is computed live
        // from the opening balance. Same kernel and inputs the freezer uses.
        let mut conn = connection::user_connection(&self.pool, user_id).await?;
        let history = projection_history::list_for_schedule_with_conn(
            &mut conn,
            user_id,
            inputs.primary_schedule.id,
        )
        .await?;
        drop(conn);
        let history: &[ProjectionHistoryRow] = if is_history_usable(&history, &inputs) {
            &history
        } else {
            &[]
        };

        let mut rows: Vec<ProjectionRow> = history.iter().map(history_row_to_projection).collect();
        rows.extend(build_rows_after(&inputs, history.last(), &reference_date));

        // Warm the per-resource caches from the lists just loaded, so the views that read them
        // right after (period view, upcoming payable) don't reload them.
        let income_active: Vec<_> = inputs
            .income_all
            .iter()
            .filter(|row| row.deleted_at.is_none())
            .cloned()
            .collect();
        self.cache.set_income(user_id, revision, income_active).await;
        self.cache
            .set_expenses(user_id, revision, inputs.expenses.clone())
            .await;
        self.cache
            .set_recurring(user_id, revision, inputs.recurring.clone())
            .await;
        self.cache
            .set_planned(user_id, revision, inputs.planned.clone())
            .await;
        self.cache
            .set_budgets(user_id, revision, inputs.budgets.clone())
            .await;

        let response = Arc::new(ProjectionsResponse {
            rows,
            primary_schedule: IncomePayScheduleResponse::from(inputs.primary_schedule),
            display_currency: inputs.display_currency,
            rates: inputs.rates,
        });

        self.cache
            .set_projections(user_id, revision, &reference_date, response.clone())
            .await;

        Ok(filter_projection_rows(response, include_past))
    }

    async fn projection_inputs(&self, user_id: Uuid) -> Result<ProjectionInputs, ApiError> {
        load_projection_inputs(&self.pool, user_id)
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest("set a primary pay schedule in settings first".into())
            })
    }

    /// The expense-item breakdown of the single projection period closing on `pay_date`. Frozen
    /// history rows store aggregates only, so the UI calls this on demand when a past period is
    /// opened. Only that one period is computed (clients read just its `expenseItems`; its balance
    /// fields carry no running total).
    pub async fn projection_period_items(
        &self,
        user_id: Uuid,
        pay_date: &str,
        as_of: Option<&str>,
    ) -> Result<Arc<ProjectionRow>, ApiError> {
        let reference_date = resolve_reference_date(as_of)?;
        let inputs = self.projection_inputs(user_id).await?;

        let schedule = schedule_from_income(&inputs.primary_schedule);
        if get_next_pay_date(&schedule, pay_date) != pay_date {
            return Err(ApiError::NotFound);
        }
        let period = get_period_for_pay_date(&schedule, pay_date);
        // Keep the opening period's mid-period start, so its items match the projection's row.
        let projection_start = inputs
            .projection_start_date
            .map(|date| date.format("%Y-%m-%d").to_string());
        if projection_start
            .as_deref()
            .is_some_and(|start| start > period.end_date.as_str())
        {
            return Err(ApiError::NotFound);
        }
        let start = projection_start
            .filter(|start| start.as_str() > period.start_date.as_str())
            .unwrap_or(period.start_date);

        let row = build_projection_rows(&inputs, 0, Some(&start), Some(pay_date), &reference_date)
            .into_iter()
            .find(|row| row.pay_date == pay_date)
            .ok_or(ApiError::NotFound)?;
        Ok(Arc::new(row))
    }

    pub async fn expense_period_view(
        &self,
        user_id: Uuid,
        period: &str,
        include_projected: bool,
        as_of: Option<&str>,
    ) -> Result<Arc<ExpensePeriodViewResponse>, ApiError> {
        let period_key = ExpensePeriodKey::parse(period).ok_or_else(|| {
            ApiError::BadRequest("invalid period; use last-period, last-month, or last-3-months".into())
        })?;

        let user_settings = self.current_settings(user_id).await?;
        let revision = user_settings.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;

        if let Some(cached) = self
            .cache
            .get_expense_period_view(user_id, revision, period, include_projected, &reference_date)
            .await
        {
            return Ok(cached);
        }

        let rates = get_exchange_rates(&self.pool, false).await?;
        let display_currency = user_settings.display_currency;

        let primary_schedule = if let Some(schedule_id) = user_settings.primary_schedule_id {
            let mut conn = connection::user_connection(&self.pool, user_id).await?;
            income_schedules::find_by_id_with_conn(&mut conn, user_id, schedule_id).await?
        } else {
            None
        };

        let expense_rows = self.expenses_with_tags(user_id, revision).await?;
        let recurring = self.recurring_with_tags(user_id, revision).await?;
        let planned = self.planned_with_tags(user_id, revision).await?;
        let budgets = self.budgets_with_tags_and_spent(user_id, revision).await?;

        let response = build_expense_period_view(
            period_key,
            primary_schedule.as_ref(),
            &expense_rows,
            &recurring,
            &planned,
            &budgets,
            display_currency,
            &rates,
            &reference_date,
            include_projected,
            user_settings.extra_spent_limit,
        )
        .ok_or_else(|| {
            ApiError::BadRequest("set a primary pay schedule in settings for pay-period view".into())
        })?;
        let response = Arc::new(response);

        self.cache
            .set_expense_period_view(
                user_id,
                revision,
                period,
                include_projected,
                &reference_date,
                response.clone(),
            )
            .await;

        Ok(response)
    }

    pub async fn upcoming_payable(
        &self,
        user_id: Uuid,
        horizon_days: i32,
        as_of: Option<&str>,
    ) -> Result<Arc<Vec<PayableFutureItem>>, ApiError> {
        let user_settings = self.current_settings(user_id).await?;
        let revision = user_settings.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;

        if let Some(cached) = self
            .cache
            .get_upcoming_payable(user_id, revision, horizon_days, &reference_date)
            .await
        {
            return Ok(cached);
        }

        let expense_rows = self.expenses_with_tags(user_id, revision).await?;
        let recurring = self.recurring_with_tags(user_id, revision).await?;
        let planned = self.planned_with_tags(user_id, revision).await?;

        let items = Arc::new(build_upcoming_payable_items(
            &expense_rows,
            &recurring,
            &planned,
            &reference_date,
            horizon_days,
        ));

        self.cache
            .set_upcoming_payable(user_id, revision, horizon_days, &reference_date, items.clone())
            .await;

        Ok(items)
    }
}

/// Maps a frozen history row to a projection row. Past periods carry aggregates only; the UI
/// fetches the per-item breakdown for an opened past period via a separate query.
fn history_row_to_projection(row: &ProjectionHistoryRow) -> ProjectionRow {
    ProjectionRow {
        pay_date: row.pay_date.format("%Y-%m-%d").to_string(),
        start_date: row.start_date.format("%Y-%m-%d").to_string(),
        end_date: row.end_date.format("%Y-%m-%d").to_string(),
        income_total: row.income,
        expense_total: row.planned_spent,
        period_free: row.free,
        cumulative_free: row.cumulative,
        expense_items: Vec::new(),
        is_past: true,
    }
}

/// Returns the shared projection response untouched when past rows are wanted (the common,
/// zero-copy path); otherwise clones once (only if the Arc is still shared) to drop past rows.
fn filter_projection_rows(
    response: Arc<ProjectionsResponse>,
    include_past: bool,
) -> Arc<ProjectionsResponse> {
    if include_past {
        return response;
    }
    let mut owned = Arc::unwrap_or_clone(response);
    owned.rows.retain(|row| !row.is_past);
    Arc::new(owned)
}
