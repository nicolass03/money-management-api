use std::sync::Arc;

use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::error::ApiError;
use crate::models::{
    IncomePayScheduleResponse, UserSettingsRow,
};
use crate::repos::{
    budgets, connection, expenses, income, income_schedules, planned_expenses, recurring_expenses,
    settings, tags,
};
use crate::services::exchange_rates::get_exchange_rates;
use crate::services::projections::build_projection_rows;
use crate::state::DbPool;
use crate::validation::today_iso;

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

    pub async fn user_settings(&self, user_id: Uuid) -> Result<UserSettingsRow, ApiError> {
        let row = settings::get_user_settings(&self.pool, user_id).await?;
        let revision = row.cache_revision;
        if let Some(cached) = self.cache.get_settings(user_id, revision).await {
            return Ok((*cached).clone());
        }
        self.cache
            .set_settings(user_id, revision, row.clone())
            .await;
        Ok(row)
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
    ) -> Result<MoneyContextResponse, ApiError> {
        if !force_refresh {
            if let Some(cached) = self.cache.get_money_context(user_id, revision).await {
                return Ok((*cached).clone());
            }
        }
        let user_settings = settings::get_user_settings(&self.pool, user_id).await?;
        let rates = get_exchange_rates(&self.pool, force_refresh).await?;
        let response = MoneyContextResponse {
            display_currency: user_settings.display_currency,
            rates,
        };
        if !force_refresh {
            self.cache
                .set_money_context(user_id, revision, response.clone())
                .await;
        }
        Ok(response)
    }

    pub async fn projections(&self, user_id: Uuid) -> Result<ProjectionsResponse, ApiError> {
        let user_settings = settings::get_user_settings(&self.pool, user_id).await?;
        let revision = user_settings.cache_revision;

        if let Some(cached) = self.cache.get_projections(user_id, revision).await {
            return Ok((*cached).clone());
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
        let income_entries = income::list_all_with_conn(&mut conn, user_id).await?;
        let expense_rows = expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let recurring = recurring_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let planned = planned_expenses::list_with_tags_with_conn(&mut conn, user_id).await?;
        let budget_rows =
            budgets::list_with_tags_and_spent_with_conn(&mut conn, user_id).await?;

        self.cache
            .set_income(user_id, revision, income_entries.clone())
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

        let rows = build_projection_rows(
            &primary_schedule,
            &income_entries,
            &expense_rows,
            &recurring,
            &planned,
            &budget_rows,
            user_settings.display_currency,
            &rates,
            user_settings.projection_initial_free_money,
            projection_start_ref,
            &today_iso(),
        );

        let response = ProjectionsResponse {
            rows,
            primary_schedule: IncomePayScheduleResponse::from(primary_schedule),
            display_currency: user_settings.display_currency,
            rates,
        };

        self.cache
            .set_projections(user_id, revision, response.clone())
            .await;

        Ok(response)
    }
}
