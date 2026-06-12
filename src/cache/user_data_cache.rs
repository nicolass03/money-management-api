use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::models::{
    BudgetRow, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow, UserSettingsRow,
};

use super::invalidation::InvalidationScope;
use super::resource::CacheResource;

type CacheKey = (Uuid, i64);

pub type ExpensesWithTags = Vec<(ExpenseRow, Vec<String>)>;
pub type RecurringWithTags = Vec<(RecurringExpenseRow, Vec<String>)>;
pub type PlannedWithTags = Vec<(PlannedExpenseRow, Vec<String>)>;
pub type BudgetsWithTagsAndSpent = Vec<(BudgetRow, Vec<String>, i32)>;

fn build_cache<T: Send + Sync + 'static>(max_capacity: u64) -> Cache<CacheKey, Arc<T>> {
    Cache::builder()
        .max_capacity(max_capacity)
        .time_to_live(Duration::from_secs(3600))
        .build()
}

#[derive(Clone)]
pub struct UserDataCache {
    enabled: bool,
    settings: Cache<CacheKey, Arc<UserSettingsRow>>,
    expenses: Cache<CacheKey, Arc<ExpensesWithTags>>,
    recurring: Cache<CacheKey, Arc<RecurringWithTags>>,
    planned: Cache<CacheKey, Arc<PlannedWithTags>>,
    budgets: Cache<CacheKey, Arc<BudgetsWithTagsAndSpent>>,
    income: Cache<CacheKey, Arc<Vec<IncomeRow>>>,
    schedules: Cache<CacheKey, Arc<Vec<IncomePayScheduleRow>>>,
    tags: Cache<CacheKey, Arc<Vec<String>>>,
    projections: Cache<CacheKey, Arc<ProjectionsResponse>>,
    money_context: Cache<CacheKey, Arc<MoneyContextResponse>>,
}

impl UserDataCache {
    pub fn new(enabled: bool, max_capacity: u64) -> Self {
        Self {
            enabled,
            settings: build_cache(max_capacity),
            expenses: build_cache(max_capacity),
            recurring: build_cache(max_capacity),
            planned: build_cache(max_capacity),
            budgets: build_cache(max_capacity),
            income: build_cache(max_capacity),
            schedules: build_cache(max_capacity),
            tags: build_cache(max_capacity),
            projections: build_cache(max_capacity),
            money_context: build_cache(max_capacity),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub async fn get_settings(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<UserSettingsRow>> {
        if !self.enabled {
            return None;
        }
        let key = (user_id, revision);
        if let Some(hit) = self.settings.get(&key).await {
            tracing::debug!(%user_id, revision, resource = "settings", "cache hit");
            return Some(hit);
        }
        None
    }

    pub async fn set_settings(&self, user_id: Uuid, revision: i64, value: UserSettingsRow) {
        if !self.enabled {
            return;
        }
        tracing::debug!(%user_id, revision, resource = "settings", "cache store");
        self.settings.insert((user_id, revision), Arc::new(value)).await;
    }

    pub async fn get_expenses(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<ExpensesWithTags>> {
        self.get_list(&self.expenses, user_id, revision, "expenses")
            .await
    }

    pub async fn set_expenses(&self, user_id: Uuid, revision: i64, value: ExpensesWithTags) {
        self.set_list(&self.expenses, user_id, revision, value, "expenses")
            .await;
    }

    pub async fn get_recurring(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<RecurringWithTags>> {
        self.get_list(&self.recurring, user_id, revision, "recurring")
            .await
    }

    pub async fn set_recurring(&self, user_id: Uuid, revision: i64, value: RecurringWithTags) {
        self.set_list(&self.recurring, user_id, revision, value, "recurring")
            .await;
    }

    pub async fn get_planned(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<PlannedWithTags>> {
        self.get_list(&self.planned, user_id, revision, "planned")
            .await
    }

    pub async fn set_planned(&self, user_id: Uuid, revision: i64, value: PlannedWithTags) {
        self.set_list(&self.planned, user_id, revision, value, "planned")
            .await;
    }

    pub async fn get_budgets(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<BudgetsWithTagsAndSpent>> {
        self.get_list(&self.budgets, user_id, revision, "budgets")
            .await
    }

    pub async fn set_budgets(&self, user_id: Uuid, revision: i64, value: BudgetsWithTagsAndSpent) {
        self.set_list(&self.budgets, user_id, revision, value, "budgets")
            .await;
    }

    pub async fn get_income(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<Vec<IncomeRow>>> {
        self.get_list(&self.income, user_id, revision, "income").await
    }

    pub async fn set_income(&self, user_id: Uuid, revision: i64, value: Vec<IncomeRow>) {
        self.set_list(&self.income, user_id, revision, value, "income")
            .await;
    }

    pub async fn get_schedules(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<Vec<IncomePayScheduleRow>>> {
        self.get_list(&self.schedules, user_id, revision, "schedules")
            .await
    }

    pub async fn set_schedules(
        &self,
        user_id: Uuid,
        revision: i64,
        value: Vec<IncomePayScheduleRow>,
    ) {
        self.set_list(&self.schedules, user_id, revision, value, "schedules")
            .await;
    }

    pub async fn get_tags(&self, user_id: Uuid, revision: i64) -> Option<Arc<Vec<String>>> {
        self.get_list(&self.tags, user_id, revision, "tags").await
    }

    pub async fn set_tags(&self, user_id: Uuid, revision: i64, value: Vec<String>) {
        self.set_list(&self.tags, user_id, revision, value, "tags")
            .await;
    }

    pub async fn get_projections(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<ProjectionsResponse>> {
        if !self.enabled {
            return None;
        }
        let key = (user_id, revision);
        if let Some(hit) = self.projections.get(&key).await {
            tracing::debug!(%user_id, revision, resource = "projections", "cache hit");
            return Some(hit);
        }
        tracing::debug!(%user_id, revision, resource = "projections", "cache miss");
        None
    }

    pub async fn set_projections(
        &self,
        user_id: Uuid,
        revision: i64,
        value: ProjectionsResponse,
    ) {
        tracing::debug!(%user_id, revision, resource = "projections", "cache store");
        self.projections
            .insert((user_id, revision), Arc::new(value))
            .await;
    }

    pub async fn get_money_context(
        &self,
        user_id: Uuid,
        revision: i64,
    ) -> Option<Arc<MoneyContextResponse>> {
        if !self.enabled {
            return None;
        }
        let key = (user_id, revision);
        if let Some(hit) = self.money_context.get(&key).await {
            tracing::debug!(%user_id, revision, resource = "money_context", "cache hit");
            return Some(hit);
        }
        None
    }

    pub async fn set_money_context(
        &self,
        user_id: Uuid,
        revision: i64,
        value: MoneyContextResponse,
    ) {
        if !self.enabled {
            return;
        }
        tracing::debug!(%user_id, revision, resource = "money_context", "cache store");
        self.money_context
            .insert((user_id, revision), Arc::new(value))
            .await;
    }

    pub fn invalidate(&self, scope: InvalidationScope, user_id: Uuid) {
        if !self.enabled {
            return;
        }
        for resource in scope.resources() {
            self.invalidate_resource(user_id, *resource);
        }
    }

    pub fn invalidate_all(&self, user_id: Uuid) {
        if !self.enabled {
            return;
        }
        for resource in [
            CacheResource::Settings,
            CacheResource::Expenses,
            CacheResource::Recurring,
            CacheResource::Planned,
            CacheResource::Budgets,
            CacheResource::Income,
            CacheResource::Schedules,
            CacheResource::Tags,
            CacheResource::Projections,
            CacheResource::MoneyContext,
        ] {
            self.invalidate_resource(user_id, resource);
        }
    }

    pub async fn run_pending_tasks(&self) {
        self.settings.run_pending_tasks().await;
        self.expenses.run_pending_tasks().await;
        self.recurring.run_pending_tasks().await;
        self.planned.run_pending_tasks().await;
        self.budgets.run_pending_tasks().await;
        self.income.run_pending_tasks().await;
        self.schedules.run_pending_tasks().await;
        self.tags.run_pending_tasks().await;
        self.projections.run_pending_tasks().await;
        self.money_context.run_pending_tasks().await;
    }

    fn invalidate_resource(&self, user_id: Uuid, resource: CacheResource) {
        let _ = match resource {
            CacheResource::Settings => self
                .settings
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Expenses => self
                .expenses
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Recurring => self
                .recurring
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Planned => self
                .planned
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Budgets => self
                .budgets
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Income => self
                .income
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Schedules => self
                .schedules
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Tags => self
                .tags
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::Projections => self
                .projections
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
            CacheResource::MoneyContext => self
                .money_context
                .invalidate_entries_if(move |key: &CacheKey, _| key.0 == user_id),
        };
    }

    async fn get_list<T: Send + Sync + Clone + 'static>(
        &self,
        cache: &Cache<CacheKey, Arc<T>>,
        user_id: Uuid,
        revision: i64,
        resource: &'static str,
    ) -> Option<Arc<T>> {
        if !self.enabled {
            return None;
        }
        let key = (user_id, revision);
        if let Some(hit) = cache.get(&key).await {
            tracing::debug!(%user_id, revision, resource, "cache hit");
            return Some(hit);
        }
        tracing::debug!(%user_id, revision, resource, "cache miss");
        None
    }

    async fn set_list<T: Send + Sync + 'static>(
        &self,
        cache: &Cache<CacheKey, Arc<T>>,
        user_id: Uuid,
        revision: i64,
        value: T,
        resource: &'static str,
    ) {
        if !self.enabled {
            return;
        }
        tracing::debug!(%user_id, revision, resource, "cache store");
        cache.insert((user_id, revision), Arc::new(value)).await;
    }
}
