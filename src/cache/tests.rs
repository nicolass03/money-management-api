use std::sync::Arc;

use uuid::Uuid;

use super::invalidation::InvalidationScope;
use super::resource::CacheResource;
use super::user_data_cache::UserDataCache;
use crate::models::{CurrencyCode, UserSettingsRow};

fn settings_row(user_id: Uuid, revision: i64) -> UserSettingsRow {
    UserSettingsRow {
        user_id,
        display_currency: CurrencyCode::Usd,
        language: "en".to_string(),
        primary_schedule_id: None,
        projection_initial_free_money: 0,
        projection_start_date: None,
        updated_at: chrono::Utc::now(),
        cache_revision: revision,
        extra_spent_limit: None,
    }
}

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
async fn settings_cache_miss_then_hit() {
    let cache = UserDataCache::new(true, 100);
    let user_id = Uuid::new_v4();

    assert!(cache.get_settings(user_id).await.is_none());
    cache
        .set_settings(user_id, Arc::new(settings_row(user_id, 3)))
        .await;
    let hit = cache.get_settings(user_id).await.expect("cache hit");
    assert_eq!(hit.cache_revision, 3);
}

#[tokio::test]
async fn any_invalidation_evicts_settings() {
    // The settings row carries `cache_revision`, which keys every other cache, so it must be
    // dropped after *any* mutation — even one whose scope doesn't list Settings.
    let cache = UserDataCache::new(true, 100);
    let user_id = Uuid::new_v4();

    cache
        .set_settings(user_id, Arc::new(settings_row(user_id, 3)))
        .await;
    assert!(cache.get_settings(user_id).await.is_some());

    cache.invalidate(InvalidationScope::ExpenseChange, user_id).await;

    assert!(
        cache.get_settings(user_id).await.is_none(),
        "settings must be evicted after any change so a fresh revision is read"
    );
}