-- UUID + multi-user migration
-- Default user: Supabase auth id + email for existing data

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE users (
    id UUID PRIMARY KEY,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO users (id, email) VALUES (
    '9886a71c-56d5-4cbe-a566-d762f24d0c9e',
    'nickph116@gmail.com'
);

-- ---------------------------------------------------------------------------
-- Phase 1: add user_id + uuid id columns to entity tables
-- ---------------------------------------------------------------------------

ALTER TABLE tags
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE recurring_expenses
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE planned_expenses
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE budgets
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE income_pay_schedules
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE savings
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE income
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN schedule_id_uuid UUID;

UPDATE income i
SET schedule_id_uuid = s.id_uuid
FROM income_pay_schedules s
WHERE i.schedule_id = s.id;

ALTER TABLE expenses
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN recurring_id_uuid UUID,
    ADD COLUMN planned_expense_id_uuid UUID,
    ADD COLUMN budget_id_uuid UUID;

UPDATE expenses e
SET recurring_id_uuid = r.id_uuid
FROM recurring_expenses r
WHERE e.recurring_id = r.id;

UPDATE expenses e
SET planned_expense_id_uuid = p.id_uuid
FROM planned_expenses p
WHERE e.planned_expense_id = p.id;

UPDATE expenses e
SET budget_id_uuid = b.id_uuid
FROM budgets b
WHERE e.budget_id = b.id;

ALTER TABLE exchange_rate_snapshots
    ADD COLUMN id_uuid UUID NOT NULL DEFAULT gen_random_uuid();

-- Junction tables: add uuid FK columns
ALTER TABLE expense_tags
    ADD COLUMN expense_id_uuid UUID,
    ADD COLUMN tag_id_uuid UUID;

UPDATE expense_tags et
SET expense_id_uuid = e.id_uuid,
    tag_id_uuid = t.id_uuid
FROM expenses e, tags t
WHERE et.expense_id = e.id AND et.tag_id = t.id;

ALTER TABLE recurring_expense_tags
    ADD COLUMN recurring_expense_id_uuid UUID,
    ADD COLUMN tag_id_uuid UUID;

UPDATE recurring_expense_tags ret
SET recurring_expense_id_uuid = r.id_uuid,
    tag_id_uuid = t.id_uuid
FROM recurring_expenses r, tags t
WHERE ret.recurring_expense_id = r.id AND ret.tag_id = t.id;

ALTER TABLE planned_expense_tags
    ADD COLUMN planned_expense_id_uuid UUID,
    ADD COLUMN tag_id_uuid UUID;

UPDATE planned_expense_tags pet
SET planned_expense_id_uuid = p.id_uuid,
    tag_id_uuid = t.id_uuid
FROM planned_expenses p, tags t
WHERE pet.planned_expense_id = p.id AND pet.tag_id = t.id;

ALTER TABLE budget_tags
    ADD COLUMN budget_id_uuid UUID,
    ADD COLUMN tag_id_uuid UUID;

UPDATE budget_tags bt
SET budget_id_uuid = b.id_uuid,
    tag_id_uuid = t.id_uuid
FROM budgets b, tags t
WHERE bt.budget_id = b.id AND bt.tag_id = t.id;

-- user_settings: add user_id + schedule uuid
ALTER TABLE user_settings
    ADD COLUMN user_id UUID NOT NULL DEFAULT '9886a71c-56d5-4cbe-a566-d762f24d0c9e' REFERENCES users(id),
    ADD COLUMN primary_schedule_id_uuid UUID;

UPDATE user_settings us
SET primary_schedule_id_uuid = s.id_uuid
FROM income_pay_schedules s
WHERE us.primary_schedule_id = s.id;

-- ---------------------------------------------------------------------------
-- Phase 2: drop dependent constraints and indexes
-- ---------------------------------------------------------------------------

ALTER TABLE budget_tags DROP CONSTRAINT IF EXISTS budget_tags_budget_id_budgets_id_fk;
ALTER TABLE budget_tags DROP CONSTRAINT IF EXISTS budget_tags_tag_id_tags_id_fk;
ALTER TABLE expense_tags DROP CONSTRAINT IF EXISTS expense_tags_expense_id_expenses_id_fk;
ALTER TABLE expense_tags DROP CONSTRAINT IF EXISTS expense_tags_tag_id_tags_id_fk;
ALTER TABLE recurring_expense_tags DROP CONSTRAINT IF EXISTS recurring_expense_tags_recurring_expense_id_recurring_expenses_id_fk;
ALTER TABLE recurring_expense_tags DROP CONSTRAINT IF EXISTS recurring_expense_tags_tag_id_tags_id_fk;
ALTER TABLE planned_expense_tags DROP CONSTRAINT IF EXISTS planned_expense_tags_planned_expense_id_planned_expenses_id_fk;
ALTER TABLE planned_expense_tags DROP CONSTRAINT IF EXISTS planned_expense_tags_tag_id_tags_id_fk;
ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_budget_id_budgets_id_fk;
ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_planned_expense_id_planned_expenses_id_fk;
ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_recurring_id_recurring_expenses_id_fk;
ALTER TABLE income DROP CONSTRAINT IF EXISTS income_schedule_id_income_pay_schedules_id_fk;
ALTER TABLE user_settings DROP CONSTRAINT IF EXISTS user_settings_primary_schedule_id_income_pay_schedules_id_fk;

DROP INDEX IF EXISTS income_scheduled_schedule_date_unique;
DROP INDEX IF EXISTS income_schedule_id_idx;
DROP INDEX IF EXISTS expenses_date_idx;
DROP INDEX IF EXISTS income_date_idx;
DROP INDEX IF EXISTS savings_date_idx;
DROP INDEX IF EXISTS expenses_recurring_id_date_idx;
DROP INDEX IF EXISTS user_settings_primary_schedule_id_idx;
DROP INDEX IF EXISTS expenses_recurring_due_unique;
DROP INDEX IF EXISTS expenses_planned_expense_id_unique;

ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_budget_exclusive;
ALTER TABLE expenses DROP CONSTRAINT IF EXISTS expenses_single_origin_chk;
ALTER TABLE income DROP CONSTRAINT IF EXISTS income_scheduled_consistency_chk;
ALTER TABLE tags DROP CONSTRAINT IF EXISTS tags_name_unique;

ALTER TABLE budget_tags DROP CONSTRAINT IF EXISTS budget_tags_budget_id_tag_id_pk;
ALTER TABLE expense_tags DROP CONSTRAINT IF EXISTS expense_tags_expense_id_tag_id_pk;
ALTER TABLE recurring_expense_tags DROP CONSTRAINT IF EXISTS recurring_expense_tags_recurring_expense_id_tag_id_pk;
ALTER TABLE planned_expense_tags DROP CONSTRAINT IF EXISTS planned_expense_tags_planned_expense_id_tag_id_pk;

-- ---------------------------------------------------------------------------
-- Phase 3: swap integer columns for UUID columns
-- ---------------------------------------------------------------------------

-- PK constraint names may differ from table names (e.g. expense_pay_schedules_pkey on recurring_expenses)
CREATE OR REPLACE FUNCTION drop_table_pkey(tbl regclass) RETURNS void AS $$
DECLARE
    constraint_name text;
BEGIN
    SELECT conname INTO constraint_name
    FROM pg_constraint
    WHERE conrelid = tbl AND contype = 'p';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I', tbl, constraint_name);
    END IF;
END;
$$ LANGUAGE plpgsql;

-- tags
SELECT drop_table_pkey('tags'::regclass);
ALTER TABLE tags DROP COLUMN id;
ALTER TABLE tags RENAME COLUMN id_uuid TO id;
ALTER TABLE tags ADD PRIMARY KEY (id);
ALTER TABLE tags ALTER COLUMN user_id DROP DEFAULT;

-- recurring_expenses
SELECT drop_table_pkey('recurring_expenses'::regclass);
ALTER TABLE recurring_expenses DROP COLUMN id;
ALTER TABLE recurring_expenses RENAME COLUMN id_uuid TO id;
ALTER TABLE recurring_expenses ADD PRIMARY KEY (id);
ALTER TABLE recurring_expenses ALTER COLUMN user_id DROP DEFAULT;

-- planned_expenses
SELECT drop_table_pkey('planned_expenses'::regclass);
ALTER TABLE planned_expenses DROP COLUMN id;
ALTER TABLE planned_expenses RENAME COLUMN id_uuid TO id;
ALTER TABLE planned_expenses ADD PRIMARY KEY (id);
ALTER TABLE planned_expenses ALTER COLUMN user_id DROP DEFAULT;

-- budgets
SELECT drop_table_pkey('budgets'::regclass);
ALTER TABLE budgets DROP COLUMN id;
ALTER TABLE budgets RENAME COLUMN id_uuid TO id;
ALTER TABLE budgets ADD PRIMARY KEY (id);
ALTER TABLE budgets ALTER COLUMN user_id DROP DEFAULT;

-- income_pay_schedules
SELECT drop_table_pkey('income_pay_schedules'::regclass);
ALTER TABLE income_pay_schedules DROP COLUMN id;
ALTER TABLE income_pay_schedules RENAME COLUMN id_uuid TO id;
ALTER TABLE income_pay_schedules ADD PRIMARY KEY (id);
ALTER TABLE income_pay_schedules ALTER COLUMN user_id DROP DEFAULT;

-- savings
SELECT drop_table_pkey('savings'::regclass);
ALTER TABLE savings DROP COLUMN id;
ALTER TABLE savings RENAME COLUMN id_uuid TO id;
ALTER TABLE savings ADD PRIMARY KEY (id);
ALTER TABLE savings ALTER COLUMN user_id DROP DEFAULT;

-- income
SELECT drop_table_pkey('income'::regclass);
ALTER TABLE income DROP COLUMN id;
ALTER TABLE income DROP COLUMN schedule_id;
ALTER TABLE income RENAME COLUMN id_uuid TO id;
ALTER TABLE income RENAME COLUMN schedule_id_uuid TO schedule_id;
ALTER TABLE income ADD PRIMARY KEY (id);
ALTER TABLE income ALTER COLUMN user_id DROP DEFAULT;

-- expenses
SELECT drop_table_pkey('expenses'::regclass);
ALTER TABLE expenses DROP COLUMN id;
ALTER TABLE expenses DROP COLUMN recurring_id;
ALTER TABLE expenses DROP COLUMN planned_expense_id;
ALTER TABLE expenses DROP COLUMN budget_id;
ALTER TABLE expenses RENAME COLUMN id_uuid TO id;
ALTER TABLE expenses RENAME COLUMN recurring_id_uuid TO recurring_id;
ALTER TABLE expenses RENAME COLUMN planned_expense_id_uuid TO planned_expense_id;
ALTER TABLE expenses RENAME COLUMN budget_id_uuid TO budget_id;
ALTER TABLE expenses ADD PRIMARY KEY (id);
ALTER TABLE expenses ALTER COLUMN user_id DROP DEFAULT;

-- exchange_rate_snapshots
SELECT drop_table_pkey('exchange_rate_snapshots'::regclass);
ALTER TABLE exchange_rate_snapshots DROP COLUMN id;
ALTER TABLE exchange_rate_snapshots RENAME COLUMN id_uuid TO id;
ALTER TABLE exchange_rate_snapshots ADD PRIMARY KEY (id);

-- expense_tags
ALTER TABLE expense_tags DROP COLUMN expense_id;
ALTER TABLE expense_tags DROP COLUMN tag_id;
ALTER TABLE expense_tags RENAME COLUMN expense_id_uuid TO expense_id;
ALTER TABLE expense_tags RENAME COLUMN tag_id_uuid TO tag_id;
ALTER TABLE expense_tags ALTER COLUMN expense_id SET NOT NULL;
ALTER TABLE expense_tags ALTER COLUMN tag_id SET NOT NULL;
ALTER TABLE expense_tags ADD PRIMARY KEY (expense_id, tag_id);

-- recurring_expense_tags
ALTER TABLE recurring_expense_tags DROP COLUMN recurring_expense_id;
ALTER TABLE recurring_expense_tags DROP COLUMN tag_id;
ALTER TABLE recurring_expense_tags RENAME COLUMN recurring_expense_id_uuid TO recurring_expense_id;
ALTER TABLE recurring_expense_tags RENAME COLUMN tag_id_uuid TO tag_id;
ALTER TABLE recurring_expense_tags ALTER COLUMN recurring_expense_id SET NOT NULL;
ALTER TABLE recurring_expense_tags ALTER COLUMN tag_id SET NOT NULL;
ALTER TABLE recurring_expense_tags ADD PRIMARY KEY (recurring_expense_id, tag_id);

-- planned_expense_tags
ALTER TABLE planned_expense_tags DROP COLUMN planned_expense_id;
ALTER TABLE planned_expense_tags DROP COLUMN tag_id;
ALTER TABLE planned_expense_tags RENAME COLUMN planned_expense_id_uuid TO planned_expense_id;
ALTER TABLE planned_expense_tags RENAME COLUMN tag_id_uuid TO tag_id;
ALTER TABLE planned_expense_tags ALTER COLUMN planned_expense_id SET NOT NULL;
ALTER TABLE planned_expense_tags ALTER COLUMN tag_id SET NOT NULL;
ALTER TABLE planned_expense_tags ADD PRIMARY KEY (planned_expense_id, tag_id);

-- budget_tags
ALTER TABLE budget_tags DROP COLUMN budget_id;
ALTER TABLE budget_tags DROP COLUMN tag_id;
ALTER TABLE budget_tags RENAME COLUMN budget_id_uuid TO budget_id;
ALTER TABLE budget_tags RENAME COLUMN tag_id_uuid TO tag_id;
ALTER TABLE budget_tags ALTER COLUMN budget_id SET NOT NULL;
ALTER TABLE budget_tags ALTER COLUMN tag_id SET NOT NULL;
ALTER TABLE budget_tags ADD PRIMARY KEY (budget_id, tag_id);

-- user_settings: user_id becomes PK
SELECT drop_table_pkey('user_settings'::regclass);
ALTER TABLE user_settings DROP COLUMN id;
ALTER TABLE user_settings DROP COLUMN primary_schedule_id;
ALTER TABLE user_settings RENAME COLUMN primary_schedule_id_uuid TO primary_schedule_id;
ALTER TABLE user_settings ADD PRIMARY KEY (user_id);
ALTER TABLE user_settings ALTER COLUMN user_id DROP DEFAULT;

-- ---------------------------------------------------------------------------
-- Phase 4: re-add foreign keys, constraints, and indexes
-- ---------------------------------------------------------------------------

ALTER TABLE tags ADD CONSTRAINT tags_user_id_name_unique UNIQUE (user_id, name);

ALTER TABLE budget_tags ADD CONSTRAINT budget_tags_budget_id_budgets_id_fk
    FOREIGN KEY (budget_id) REFERENCES budgets(id) ON DELETE CASCADE;
ALTER TABLE budget_tags ADD CONSTRAINT budget_tags_tag_id_tags_id_fk
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE;

ALTER TABLE expense_tags ADD CONSTRAINT expense_tags_expense_id_expenses_id_fk
    FOREIGN KEY (expense_id) REFERENCES expenses(id) ON DELETE CASCADE;
ALTER TABLE expense_tags ADD CONSTRAINT expense_tags_tag_id_tags_id_fk
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE;

ALTER TABLE recurring_expense_tags ADD CONSTRAINT recurring_expense_tags_recurring_expense_id_fk
    FOREIGN KEY (recurring_expense_id) REFERENCES recurring_expenses(id) ON DELETE CASCADE;
ALTER TABLE recurring_expense_tags ADD CONSTRAINT recurring_expense_tags_tag_id_tags_id_fk
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE;

ALTER TABLE planned_expense_tags ADD CONSTRAINT planned_expense_tags_planned_expense_id_fk
    FOREIGN KEY (planned_expense_id) REFERENCES planned_expenses(id) ON DELETE CASCADE;
ALTER TABLE planned_expense_tags ADD CONSTRAINT planned_expense_tags_tag_id_tags_id_fk
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE;

ALTER TABLE expenses ADD CONSTRAINT expenses_recurring_id_fk
    FOREIGN KEY (recurring_id) REFERENCES recurring_expenses(id) ON DELETE CASCADE;
ALTER TABLE expenses ADD CONSTRAINT expenses_planned_expense_id_fk
    FOREIGN KEY (planned_expense_id) REFERENCES planned_expenses(id) ON DELETE SET NULL;
ALTER TABLE expenses ADD CONSTRAINT expenses_budget_id_fk
    FOREIGN KEY (budget_id) REFERENCES budgets(id) ON DELETE SET NULL;

ALTER TABLE income ADD CONSTRAINT income_schedule_id_fk
    FOREIGN KEY (schedule_id) REFERENCES income_pay_schedules(id);

ALTER TABLE user_settings ADD CONSTRAINT user_settings_primary_schedule_id_fk
    FOREIGN KEY (primary_schedule_id) REFERENCES income_pay_schedules(id);

ALTER TABLE expenses ADD CONSTRAINT expenses_budget_exclusive
    CHECK (NOT (budget_id IS NOT NULL AND (recurring_id IS NOT NULL OR planned_expense_id IS NOT NULL)));
ALTER TABLE expenses ADD CONSTRAINT expenses_single_origin_chk
    CHECK (NOT (recurring_id IS NOT NULL AND planned_expense_id IS NOT NULL));

ALTER TABLE income ADD CONSTRAINT income_scheduled_consistency_chk
    CHECK (
        (source = 'scheduled' AND schedule_id IS NOT NULL)
        OR (source = 'manual' AND schedule_id IS NULL)
    );

CREATE UNIQUE INDEX income_scheduled_schedule_date_unique
    ON income (schedule_id, date) WHERE source = 'scheduled';
CREATE INDEX income_schedule_id_idx ON income (schedule_id);
CREATE INDEX expenses_date_idx ON expenses (date DESC);
CREATE INDEX income_date_idx ON income (date DESC);
CREATE INDEX savings_date_idx ON savings (date DESC);
CREATE INDEX expenses_recurring_id_date_idx ON expenses (recurring_id, date) WHERE recurring_id IS NOT NULL;
CREATE INDEX user_settings_primary_schedule_id_idx ON user_settings (primary_schedule_id);

CREATE UNIQUE INDEX expenses_recurring_due_unique
    ON expenses (recurring_id, COALESCE(scheduled_date, date)) WHERE recurring_id IS NOT NULL;
CREATE UNIQUE INDEX expenses_planned_expense_id_unique
    ON expenses (planned_expense_id) WHERE planned_expense_id IS NOT NULL;

CREATE INDEX tags_user_id_idx ON tags (user_id);
CREATE INDEX recurring_expenses_user_id_idx ON recurring_expenses (user_id);
CREATE INDEX planned_expenses_user_id_idx ON planned_expenses (user_id);
CREATE INDEX budgets_user_id_idx ON budgets (user_id);
CREATE INDEX income_pay_schedules_user_id_idx ON income_pay_schedules (user_id);
CREATE INDEX savings_user_id_idx ON savings (user_id);
CREATE INDEX income_user_id_idx ON income (user_id);
CREATE INDEX expenses_user_id_idx ON expenses (user_id);

DROP FUNCTION drop_table_pkey(regclass);
