use uuid::Uuid;

use super::invalidation::InvalidationScope;
use super::resource::CacheResource;
use super::user_data_cache::UserDataCache;
use crate::models::{CurrencyCode, UserSettingsRow};

#[test]
fn invalidation_scope_maps_to_resources() {
    let expense = InvalidationScope::ExpenseChange.resources();
    assert!(expense.contains(&CacheResource::Expenses));
    assert!(expense.contains(&CacheResource::Projections));

    let settings = InvalidationScope::SettingsChange.resources();
    assert!(settings.contains(&CacheResource::Settings));
    assert!(settings.contains(&CacheResource::MoneyContext));
}

#[tokio::test]
async fn cache_miss_then_hit_same_revision() {
    let cache = UserDataCache::new(true, 100);
    let user_id = Uuid::new_v4();
    let revision = 3_i64;
    let row = UserSettingsRow {
        user_id,
        display_currency: CurrencyCode::Usd,
        primary_schedule_id: None,
        projection_initial_free_money: 0,
        projection_start_date: None,
        updated_at: chrono::Utc::now(),
        cache_revision: revision,
    };

    assert!(cache.get_settings(user_id, revision).await.is_none());
    cache.set_settings(user_id, revision, row.clone()).await;
    let hit = cache
        .get_settings(user_id, revision)
        .await
        .expect("cache hit");
    assert_eq!(hit.cache_revision, revision);

    assert!(
        cache.get_settings(user_id, revision + 1).await.is_none(),
        "bumped revision must miss"
    );
}