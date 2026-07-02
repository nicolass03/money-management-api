DROP INDEX IF EXISTS recurring_expenses_account_id_idx;
ALTER TABLE recurring_expenses DROP COLUMN account_id;
