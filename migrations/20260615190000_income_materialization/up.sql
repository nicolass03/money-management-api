-- Income now materializes scheduled pay dates via the daily cron (mirrors recurring
-- expenses) instead of eagerly pre-syncing a multi-year window of future rows.
--
-- `amount_overridden` marks scheduled income whose amount was edited after
-- materialization (parity with expenses.amount_overridden).
-- `deleted_at` soft-deletes a materialized scheduled income row so the daily cron
-- (and projections) do not resurrect an occurrence the user intentionally removed.
ALTER TABLE income
    ADD COLUMN amount_overridden BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN deleted_at TIMESTAMPTZ NULL;

-- Drop pre-synced future scheduled income; the cron is now the source of truth for
-- materialized future occurrences and projections cover the rest.
DELETE FROM income
WHERE source = 'scheduled'
  AND date > CURRENT_DATE;
