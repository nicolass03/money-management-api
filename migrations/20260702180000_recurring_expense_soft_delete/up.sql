-- Soft-delete recurring expenses: cancel the schedule without erasing materialized charges.
ALTER TABLE recurring_expenses
    ADD COLUMN deleted_at TIMESTAMPTZ NULL;

-- Hard delete must not wipe linked expense history (soft delete is the only supported path).
ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_recurring_id_fk;
ALTER TABLE expenses ADD CONSTRAINT expenses_recurring_id_fk
    FOREIGN KEY (recurring_id) REFERENCES recurring_expenses(id) ON DELETE RESTRICT;
