use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use moka::future::Cache;
use uuid::Uuid;

use crate::dto::{MoneyContextResponse, ProjectionsResponse};
use crate::models::{
    BudgetRow, ExpenseRow, IncomePayScheduleRow, IncomeRow, PlannedExpenseRow,
    RecurringExpenseRow, UserSettingsRow,
};
use crate::services::expense_period::ExpensePeriodViewResponse;
use crate::services::upcoming_payable::PayableFutureItem;

pub type ExpensesWithTags = Vec<(ExpenseRow, Vec<String>)>;
pub type RecurringWithTags = Vec<(RecurringExpenseRow, Vec<String>)>;
pub type PlannedWithTags = Vec<(PlannedExpenseRow, Vec<String>)>;
pub type BudgetsWithTagsAndSpent = Vec<(BudgetRow, Vec<String>, i32)>;

/// Revision-keyed entries can never be served stale (a write bumps the revision, so old keys are
/// simply unreachable); this TTL only bounds how long an idle user's entries hold memory.
const ENTRY_TTL: Duration = Duration::from_secs(3600);

/// How long a user's settings row (which carries `cache_revision`) may live in memory without
/// being re-read. Eviction on every write keeps it correct; this short TTL is a self-healing
/// safety net, and bounds how long another replica can miss a write made elsewhere.
const SETTINGS_TTL: Duration = Duration::from_secs(60);

/// One resource's cache. Keys start with the owning user's id; `K` is the rest of the key — the
/// `cache_revision` plus any view parameters (`today` is included for day-relative views so their
/// boundaries don't go stale across midnight while the revision is unchanged).
pub struct UserCache<K, V> {
    inner: Cache<(Uuid, K), Arc<V>>,
    enabled: bool,
    resource: &'static str,
}

impl<K, V> UserCache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    fn new(resource: &'static str, enabled: bool, max_capacity: u64, ttl: Duration) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(ttl)
                .build(),
            enabled,
            resource,
        }
    }

    pub async fn get(&self, user_id: Uuid, key: K) -> Option<Arc<V>> {
        if !self.enabled {
            return None;
        }
        let hit = self.inner.get(&(user_id, key)).await;
        tracing::debug!(%user_id, resource = self.resource, hit = hit.is_some(), "cache lookup");
        hit
    }

    pub async fn insert(&self, user_id: Uuid, key: K, value: Arc<V>) {
        if self.enabled {
            self.inner.insert((user_id, key), value).await;
        }
    }
}

type Revision = i64;

pub struct UserDataCache {
    // Keyed by `user_id` alone (not revision): this is what lets a warm request obtain the current
    // `cache_revision` without a database round trip. Everything else is keyed by revision.
    pub settings: UserCache<(), UserSettingsRow>,
    pub expenses: UserCache<Revision, ExpensesWithTags>,
    pub recurring: UserCache<Revision, RecurringWithTags>,
    pub planned: UserCache<Revision, PlannedWithTags>,
    pub budgets: UserCache<Revision, BudgetsWithTagsAndSpent>,
    pub income: UserCache<Revision, Vec<IncomeRow>>,
    pub schedules: UserCache<Revision, Vec<IncomePayScheduleRow>>,
    pub tags: UserCache<Revision, Vec<String>>,
    pub money_context: UserCache<Revision, MoneyContextResponse>,
    /// `(revision, as_of)`.
    pub projections: UserCache<(Revision, NaiveDate), ProjectionsResponse>,
    /// `(revision, period, include_projected, as_of)`.
    pub expense_period_view:
        UserCache<(Revision, String, bool, NaiveDate), ExpensePeriodViewResponse>,
    /// `(revision, horizon_days, as_of)`.
    pub upcoming_payable: UserCache<(Revision, i32, NaiveDate), Vec<PayableFutureItem>>,
}

impl UserDataCache {
    pub fn new(enabled: bool, max_capacity: u64) -> Self {
        Self {
            settings: UserCache::new("settings", enabled, max_capacity, SETTINGS_TTL),
            expenses: UserCache::new("expenses", enabled, max_capacity, ENTRY_TTL),
            recurring: UserCache::new("recurring", enabled, max_capacity, ENTRY_TTL),
            planned: UserCache::new("planned", enabled, max_capacity, ENTRY_TTL),
            budgets: UserCache::new("budgets", enabled, max_capacity, ENTRY_TTL),
            income: UserCache::new("income", enabled, max_capacity, ENTRY_TTL),
            schedules: UserCache::new("schedules", enabled, max_capacity, ENTRY_TTL),
            tags: UserCache::new("tags", enabled, max_capacity, ENTRY_TTL),
            money_context: UserCache::new("money_context", enabled, max_capacity, ENTRY_TTL),
            projections: UserCache::new("projections", enabled, max_capacity, ENTRY_TTL),
            expense_period_view: UserCache::new(
                "expense_period_view",
                enabled,
                max_capacity,
                ENTRY_TTL,
            ),
            upcoming_payable: UserCache::new("upcoming_payable", enabled, max_capacity, ENTRY_TTL),
        }
    }

    /// Call after every committed write. Every write bumps `cache_revision`, so evicting the
    /// cached settings row (which carries the revision) is all it takes: the next read sees the
    /// new revision and every other cache misses by key. This must be a precise, immediate
    /// eviction — a mutate-then-refetch can happen within the same millisecond.
    pub async fn invalidate(&self, user_id: Uuid) {
        self.settings.inner.invalidate(&(user_id, ())).await;
    }
}
