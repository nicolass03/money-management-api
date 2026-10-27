-- Drop indexes already covered by a unique index with the same leading column(s).
DROP INDEX IF EXISTS projection_history_user_schedule_idx; -- = UNIQUE (user_id, schedule_id, pay_date)
DROP INDEX IF EXISTS projection_history_user_id_idx;       -- leading column of that unique index
DROP INDEX IF EXISTS tags_user_id_idx;                     -- leading column of UNIQUE (user_id, name)

-- Index the budget FK: per-budget spend sums and its ON DELETE SET NULL look rows up by it.
CREATE INDEX IF NOT EXISTS expenses_budget_id_idx ON expenses (budget_id) WHERE budget_id IS NOT NULL;
