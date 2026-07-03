-- Retire user_settings.projection_initial_free_money. The projection's opening balance is now
-- seeded solely from account initial amounts (see cache/loader.rs and services/projection_history.rs);
-- the legacy single "free money" value was migrated into each user's Default account back in
-- 20260630120000_add_accounts, so no live data is lost by dropping it.
ALTER TABLE user_settings DROP COLUMN projection_initial_free_money;
