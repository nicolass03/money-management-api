ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_recurring_id_fk;
ALTER TABLE expenses ADD CONSTRAINT expenses_recurring_id_fk
    FOREIGN KEY (recurring_id) REFERENCES recurring_expenses(id) ON DELETE CASCADE;

ALTER TABLE recurring_expenses DROP COLUMN deleted_at;
