-- Re-add the column (data cannot be restored; it lived on since 20260630120000_add_accounts).
ALTER TABLE user_settings ADD COLUMN projection_initial_free_money INT4 NOT NULL DEFAULT 0;
