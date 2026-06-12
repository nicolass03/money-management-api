ALTER TABLE user_settings
    ADD COLUMN cache_revision BIGINT NOT NULL DEFAULT 0;
