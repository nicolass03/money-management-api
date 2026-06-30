-- Accounts: users can hold money across multiple accounts (e.g. cash, EUR account, USD account),
-- each with an optional name, a currency, and an initial amount. Expenses draw from an account and
-- income lands in one; recurring-expense charges pick an account by currency at charge time.

CREATE TABLE accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT,
    currency currency_code NOT NULL,
    initial_amount INT4 NOT NULL DEFAULT 0,
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX accounts_user_id_idx ON accounts (user_id);

-- Row-level security, mirroring the other user-owned tables (see 20250612130000_row_level_security).
ALTER TABLE accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounts_tenant ON accounts
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

-- An account reference on every money-bearing row the user assigns directly. Nullable for backward
-- compatibility; ON DELETE SET NULL so archiving/removing an account never destroys history.
-- recurring_expenses is intentionally excluded: the daily charge job picks the account by currency.
ALTER TABLE expenses ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE income ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE income_pay_schedules ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE planned_expenses ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;

CREATE INDEX expenses_account_id_idx ON expenses (account_id);
CREATE INDEX income_account_id_idx ON income (account_id);
CREATE INDEX income_pay_schedules_account_id_idx ON income_pay_schedules (account_id);
CREATE INDEX planned_expenses_account_id_idx ON planned_expenses (account_id);

-- Backfill: give every existing user a default account in their display currency, seeded from the
-- single starting balance they had until now (user_settings.projection_initial_free_money), then
-- attach all of their existing rows to it so balances and projections reconcile.
INSERT INTO accounts (user_id, name, currency, initial_amount)
SELECT user_id, 'Default', display_currency, projection_initial_free_money
FROM user_settings;

UPDATE expenses e
SET account_id = a.id
FROM accounts a
WHERE a.user_id = e.user_id AND a.name = 'Default' AND e.account_id IS NULL;

UPDATE income i
SET account_id = a.id
FROM accounts a
WHERE a.user_id = i.user_id AND a.name = 'Default' AND i.account_id IS NULL;

UPDATE income_pay_schedules s
SET account_id = a.id
FROM accounts a
WHERE a.user_id = s.user_id AND a.name = 'Default' AND s.account_id IS NULL;

UPDATE planned_expenses p
SET account_id = a.id
FROM accounts a
WHERE a.user_id = p.user_id AND a.name = 'Default' AND p.account_id IS NULL;
