use std::sync::Arc;

use uuid::Uuid;

use super::user_data_cache::UserDataCache;
use crate::models::{CurrencyCode, UserSettingsRow};

fn settings_row(user_id: Uuid, revision: i64) -> UserSettingsRow {
    UserSettingsRow {
        user_id,
        display_currency: CurrencyCode::Usd,
        language: "en".to_string(),
        primary_schedule_id: None,
        projection_start_date: None,
        projection_end_date: None,
        updated_at: chrono::Utc::now(),
        cache_revision: revision,
        extra_spent_limit: None,
        theme: "dark".to_string(),
    }
}

#[tokio::test]
async fn settings_cache_miss_then_hit() {
    let cache = UserDataCache::new(true, 100);
    let user_id = Uuid::new_v4();

    assert!(cache.settings.get(user_id, ()).await.is_none());
    cache
        .settings
        .insert(user_id, (), Arc::new(settings_row(user_id, 3)))
        .await;
    let hit = cache.settings.get(user_id, ()).await.expect("cache hit");
    assert_eq!(hit.cache_revision, 3);
}

#[tokio::test]
async fn invalidation_evicts_settings() {
    // The settings row carries `cache_revision`, which keys every other cache, so it must be
    // dropped after any write.
    let cache = UserDataCache::new(true, 100);
    let user_id = Uuid::new_v4();

    cache
        .settings
        .insert(user_id, (), Arc::new(settings_row(user_id, 3)))
        .await;
    assert!(cache.settings.get(user_id, ()).await.is_some());

    cache.invalidate(user_id).await;

    assert!(
        cache.settings.get(user_id, ()).await.is_none(),
        "settings must be evicted after any change so a fresh revision is read"
    );
}
