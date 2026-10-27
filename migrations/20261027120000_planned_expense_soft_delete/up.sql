-- Soft-delete one-time (planned) expenses. A hard delete set the payment's `planned_expense_id` to
-- NULL (ON DELETE SET NULL), which silently turned a planned payment into manual "extra" spend.
-- Keeping the row (tombstoned) keeps the link, like recurring expenses and pay schedules.
ALTER TABLE planned_expenses
    ADD COLUMN deleted_at TIMESTAMPTZ NULL;
