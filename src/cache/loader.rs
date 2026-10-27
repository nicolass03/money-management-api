use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::error::ApiError;
use crate::models::{
    IncomePayScheduleResponse, IncomePayScheduleRow, IncomeRow, ProjectionHistoryRow,
    UserSettingsRow,
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

use super::user_data_cache::{
    BudgetsWithTagsAndSpent, ExpensesWithTags, PlannedWithTags, RecurringWithTags, UserCache,
    UserDataCache,
};

#[derive(Clone)]
pub struct UserDataLoader {
    pool: DbPool,
    cache: Arc<UserDataCache>,
}

/// Serves `key` from `cache`, else runs `load`, caches its result and returns it. Values are
/// shared (`Arc`), so a hit never copies the data.
async fn cached<K, V>(
    cache: &UserCache<K, V>,
    user_id: Uuid,
    key: K,
    load: impl Future<Output = Result<V, ApiError>>,
) -> Result<Arc<V>, ApiError>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    if let Some(hit) = cache.get(user_id, key.clone()).await {
        return Ok(hit);
    }
    let value = Arc::new(load.await?);
    cache.insert(user_id, key, value.clone()).await;
    Ok(value)
}

impl UserDataLoader {
    pub fn new(pool: DbPool, cache: Arc<UserDataCache>) -> Self {
        Self { pool, cache }
    }

    /// The user's settings row — carrying the current `cache_revision` that keys every other
    /// cache — served from memory on a warm path so cache-backed reads need **zero** database
    /// round trips. Evicted on every write (see `UserDataCache::invalidate`).
    pub async fn user_settings(&self, user_id: Uuid) -> Result<Arc<UserSettingsRow>, ApiError> {
        cached(
            &self.cache.settings,
            user_id,
            (),
            settings::get_user_settings(&self.pool, user_id),
        )
        .await
    }

    pub async fn expenses_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<ExpensesWithTags>, ApiError> {
        let load = expenses::list_with_tags(&self.pool, user_id);
        cached(&self.cache.expenses, user_id, revision, load).await
    }

    pub async fn recurring_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<RecurringWithTags>, ApiError> {
        let load = recurring_expenses::list_with_tags(&self.pool, user_id);
        cached(&self.cache.recurring, user_id, revision, load).await
    }

    pub async fn planned_with_tags(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<PlannedWithTags>, ApiError> {
        let load = planned_expenses::list_with_tags(&self.pool, user_id);
        cached(&self.cache.planned, user_id, revision, load).await
    }

    pub async fn budgets_with_tags_and_spent(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<BudgetsWithTagsAndSpent>, ApiError> {
        let load = budgets::list_with_tags_and_spent(&self.pool, user_id);
        cached(&self.cache.budgets, user_id, revision, load).await
    }

    /// Active (non-deleted) income rows.
    pub async fn income_list(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<Vec<IncomeRow>>, ApiError> {
        let load = income::list_all(&self.pool, user_id);
        cached(&self.cache.income, user_id, revision, load).await
    }

    pub async fn schedules_list(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Result<Arc<Vec<IncomePayScheduleRow>>, ApiError> {
        let load = income_schedules::list_all(&self.pool, user_id);
        cached(&self.cache.schedules, user_id, revision, load).await
    }

    /// The user's primary pay schedule, if set (from the cached schedule list).
    pub async fn primary_schedule(
        &self,
        user_id: Uuid,
        settings: &UserSettingsRow,
    ) -> Result<Option<IncomePayScheduleRow>, ApiError> {
        let Some(schedule_id) = settings.primary_schedule_id else {
            return Ok(None);
        };
        let schedules = self.schedules_list(user_id, settings.cache_revision).await?;
        Ok(schedules.iter().find(|schedule| schedule.id == schedule_id).cloned())
    }

    pub async fn tag_names(&self, user_id: Uuid, revision: i64) -> Result<Arc<Vec<String>>, ApiError> {
        let load = async {
            let mut conn = connection::user_connection(&self.pool, user_id).await?;
            Ok(tags::list_all_names(&mut conn, user_id).await?)
        };
        cached(&self.cache.tags, user_id, revision, load).await
    }

    /// Display currency + exchange rates. A forced refresh refetches the rates and replaces the
    /// cached entry, so later non-forced reads don't keep serving the old rates.
    pub async fn money_context(
        &self,
        user_id: Uuid,
        force_refresh: bool,
    ) -> Result<Arc<MoneyContextResponse>, ApiError> {
        let user_settings = self.user_settings(user_id).await?;
        let revision = user_settings.cache_revision;
        if !force_refresh {
            if let Some(hit) = self.cache.money_context.get(user_id, revision).await {
                return Ok(hit);
            }
        }
        let response = Arc::new(MoneyContextResponse {
            display_currency: user_settings.display_currency,
            rates: get_exchange_rates(&self.pool, force_refresh).await?,
        });
        self.cache
            .money_context
            .insert(user_id, revision, response.clone())
            .await;
        Ok(response)
    }

    pub async fn projections(
        &self,
        user_id: Uuid,
        include_past: bool,
        as_of: Option<&str>,
    ) -> Result<Arc<ProjectionsResponse>, ApiError> {
        let revision = self.user_settings(user_id).await?.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;
        let key = (revision, reference_date);

        if let Some(hit) = self.cache.projections.get(user_id, key).await {
            return Ok(filter_projection_rows(hit, include_past));
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
        rows.extend(build_rows_after(&inputs, history.last(), reference_date));

        // Warm the per-resource caches with the lists just loaded, so the views that read them
        // right after (period view, upcoming payable) don't reload them.
        let ProjectionInputs {
            primary_schedule,
            income_all,
            expenses,
            recurring,
            planned,
            budgets,
            display_currency,
            rates,
            ..
        } = inputs;
        let income_active: Vec<IncomeRow> = income_all
            .into_iter()
            .filter(|row| row.deleted_at.is_none())
            .collect();
        let cache = &self.cache;
        cache.income.insert(user_id, revision, Arc::new(income_active)).await;
        cache.expenses.insert(user_id, revision, Arc::new(expenses)).await;
        cache.recurring.insert(user_id, revision, Arc::new(recurring)).await;
        cache.planned.insert(user_id, revision, Arc::new(planned)).await;
        cache.budgets.insert(user_id, revision, Arc::new(budgets)).await;

        let response = Arc::new(ProjectionsResponse {
            rows,
            primary_schedule: IncomePayScheduleResponse::from(primary_schedule),
            display_currency,
            rates,
        });
        cache.projections.insert(user_id, key, response.clone()).await;

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
        pay_date: NaiveDate,
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
        if inputs
            .projection_start_date
            .is_some_and(|start| start > period.end_date)
        {
            return Err(ApiError::NotFound);
        }
        let start = inputs
            .projection_start_date
            .filter(|start| *start > period.start_date)
            .unwrap_or(period.start_date);

        let row = build_projection_rows(&inputs, 0, Some(start), Some(pay_date), reference_date)
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

        let user_settings = self.user_settings(user_id).await?;
        let revision = user_settings.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;
        let key = (revision, period.to_string(), include_projected, reference_date);

        let load = async {
            let rates = get_exchange_rates(&self.pool, false).await?;
            let primary_schedule = self.primary_schedule(user_id, &user_settings).await?;
            let expense_rows = self.expenses_with_tags(user_id, revision).await?;
            let recurring = self.recurring_with_tags(user_id, revision).await?;
            let planned = self.planned_with_tags(user_id, revision).await?;
            let budgets = self.budgets_with_tags_and_spent(user_id, revision).await?;

            build_expense_period_view(
                period_key,
                primary_schedule.as_ref(),
                &expense_rows,
                &recurring,
                &planned,
                &budgets,
                user_settings.display_currency,
                &rates,
                reference_date,
                include_projected,
                user_settings.extra_spent_limit,
            )
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "set a primary pay schedule in settings for pay-period view".into(),
                )
            })
        };
        cached(&self.cache.expense_period_view, user_id, key, load).await
    }

    pub async fn upcoming_payable(
        &self,
        user_id: Uuid,
        horizon_days: i32,
        as_of: Option<&str>,
    ) -> Result<Arc<Vec<PayableFutureItem>>, ApiError> {
        let revision = self.user_settings(user_id).await?.cache_revision;
        let reference_date = resolve_reference_date(as_of)?;
        let key = (revision, horizon_days, reference_date);

        let load = async {
            let expense_rows = self.expenses_with_tags(user_id, revision).await?;
            let recurring = self.recurring_with_tags(user_id, revision).await?;
            let planned = self.planned_with_tags(user_id, revision).await?;
            Ok(build_upcoming_payable_items(
                &expense_rows,
                &recurring,
                &planned,
                reference_date,
                horizon_days,
            ))
        };
        cached(&self.cache.upcoming_payable, user_id, key, load).await
    }
}

/// Maps a frozen history row to a projection row. Past periods carry aggregates only; the UI
/// fetches the per-item breakdown for an opened past period via a separate query.
fn history_row_to_projection(row: &ProjectionHistoryRow) -> ProjectionRow {
    ProjectionRow {
        pay_date: row.pay_date,
        start_date: row.start_date,
        end_date: row.end_date,
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
