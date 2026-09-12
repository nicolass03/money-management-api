use std::sync::Arc;

use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::error::ApiError;
use crate::models::{
    IncomePayScheduleResponse, ProjectionHistoryRow, UserSettingsRow,
};
use crate::repos::{
    accounts, budgets, connection, expenses, income, income_schedules, planned_expenses,
    projection_history, recurring_expenses, settings, tags,
};
use crate::services::currency::convert_amount;
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::expense_period::{
    build_expense_period_view, ExpensePeriodKey, ExpensePeriodViewResponse,
};
use crate::services::projections::{build_projection_rows, ProjectionRow};
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

        let Some(schedule_id) = user_settings.primary_schedule_id else {
            return Err(ApiError::BadRequest(
                "set a primary pay schedule in settings first".into(),
            ));
        };

        let mut conn = connection::user_connection(&self.pool, user_id).await?;
        let Some(primary_schedule) =
            income_schedules::find_by_id_with_conn(&mut conn, user_id, schedule_id).await?
        else {
            return Err(ApiError::BadRequest("primary schedule not found".into()));
        };

        let rates = get_exchange_rates(&self.pool, false).await?;
        // Projections need every pay schedule (income can come from non-primary schedules)
        // and tombstoned rows so deleted scheduled occurrences are not re-projected.
        let schedules = income_schedules::list_all(&self.pool, user_id).await?;
        let income_all = income::list_with_deleted_with_conn(&mut conn, user_id).await?;
        let income_active: Vec<crate::models::IncomeRow> = income_all
            .iter()
            .filter(|row| row.deleted_at.is_none())
            .cloned()
            .collect();
        let expense_rows = expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let recurring = recurring_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let planned = planned_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let budget_rows =
            budgets::list_with_tags_and_spent_with_conn(&mut conn, user_id).await?;

        self.cache
            .set_income(user_id, revision, income_active)
            .await;
        self.cache
            .set_expenses(user_id, revision, expense_rows.clone())
            .await;
        self.cache
            .set_recurring(user_id, revision, recurring.clone())
            .await;
        self.cache
            .set_planned(user_id, revision, planned.clone())
            .await;
        self.cache
            .set_budgets(user_id, revision, budget_rows.clone())
            .await;

        let projection_start_date = user_settings
            .projection_start_date
            .map(|d| d.format("%Y-%m-%d").to_string());
        let projection_start_ref = projection_start_date.as_deref();
        let projection_end_date = user_settings
            .projection_end_date
            .map(|d| d.format("%Y-%m-%d").to_string());
        let projection_end_ref = projection_end_date.as_deref();

        // Opening balance = sum of every account's initial amount, converted into the display
        // currency. Accounts are the single source of starting money (they replaced the legacy
        // `user_settings.projection_initial_free_money`, which is no longer added here — doing so
        // double-counted the seed). Archived accounts are included: their historical expense/income
        // rows are still counted in the projection, so their initial amount must stay in the seed to
        // keep the running balance continuous when an account is archived.
        let account_list = accounts::list_all_with_conn(&mut conn, user_id).await?;
        let initial_free_money: i32 = account_list
            .iter()
            .map(|account| {
                convert_amount(
                    account.initial_amount,
                    account.currency,
                    user_settings.display_currency,
                    &rates,
                )
            })
            .sum();

        // Past periods are served from the frozen history table; only the current + future periods
        // are computed live, seeded from the balance carried by the frozen rows. On a fresh user
        // with nothing frozen yet (or a display-currency mismatch pending re-init), fall back to a
        // full live computation from the opening balance — the original behavior.
        let history =
            projection_history::list_for_schedule_with_conn(&mut conn, user_id, schedule_id).await?;
        let history_usable = !history.is_empty()
            && history
                .iter()
                .all(|row| row.currency == user_settings.display_currency);

        let rows = if history_usable {
            let mut rows: Vec<ProjectionRow> =
                history.iter().map(history_row_to_projection).collect();
            // The current period takes the last frozen entry as its base: seed the live run from
            // that row's cumulative and start at the period right after it (which also self-heals
            // any period the daily job hasn't frozen yet — it re-appears as a live past row).
            let last = history.last().expect("history is non-empty when usable");
            let seed = last.cumulative;
            let range_start = (last.pay_date + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();
            let live = build_projection_rows(
                &primary_schedule,
                &schedules,
                &income_all,
                &expense_rows,
                &recurring,
                &planned,
                &budget_rows,
                user_settings.display_currency,
                &rates,
                seed,
                Some(&range_start),
                projection_end_ref,
                &reference_date,
            );
            rows.extend(live);
            rows
        } else {
            build_projection_rows(
                &primary_schedule,
                &schedules,
                &income_all,
                &expense_rows,
                &recurring,
                &planned,
                &budget_rows,
                user_settings.display_currency,
                &rates,
                initial_free_money,
                projection_start_ref,
                projection_end_ref,
                &reference_date,
            )
        };

        let response = Arc::new(ProjectionsResponse {
            rows,
            primary_schedule: IncomePayScheduleResponse::from(primary_schedule),
            display_currency: user_settings.display_currency,
            rates,
        });

        self.cache
            .set_projections(user_id, revision, &reference_date, response.clone())
            .await;

        Ok(filter_projection_rows(response, include_past))
    }

    /// The single projection period closing on `pay_date`, computed fresh with its full expense
    /// item breakdown. Frozen history rows store aggregates only, so the UI calls this on demand
    /// when a past period is opened. Recomputed with the same kernel as `projections`, so the item
    /// total matches the frozen `planned_spent` for that period.
    pub async fn projection_period_items(
        &self,
        user_id: Uuid,
        pay_date: &str,
        as_of: Option<&str>,
    ) -> Result<Arc<ProjectionRow>, ApiError> {
        let reference_date = resolve_reference_date(as_of)?;
        let Some(inputs) =
            crate::services::projection_history::load_projection_inputs(&self.pool, user_id).await?
        else {
            return Err(ApiError::BadRequest(
                "set a primary pay schedule in settings first".into(),
            ));
        };

        let projection_start = inputs
            .projection_start_date
            .map(|date| date.format("%Y-%m-%d").to_string());
        let projection_end = inputs
            .projection_end_date
            .map(|date| date.format("%Y-%m-%d").to_string());
        let rows = build_projection_rows(
            &inputs.primary_schedule,
            &inputs.schedules,
            &inputs.income_all,
            &inputs.expenses,
            &inputs.recurring,
            &inputs.planned,
            &inputs.budgets,
            inputs.display_currency,
            &inputs.rates,
            inputs.initial_free_money,
            projection_start.as_deref(),
            projection_end.as_deref(),
            &reference_date,
        );

        let row = rows
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
