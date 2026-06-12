DROP POLICY IF EXISTS exchange_rate_snapshots_all ON exchange_rate_snapshots;
ALTER TABLE exchange_rate_snapshots DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS recurring_expense_tags_tenant ON recurring_expense_tags;
ALTER TABLE recurring_expense_tags DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS planned_expense_tags_tenant ON planned_expense_tags;
ALTER TABLE planned_expense_tags DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS expense_tags_tenant ON expense_tags;
ALTER TABLE expense_tags DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS budget_tags_tenant ON budget_tags;
ALTER TABLE budget_tags DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS user_settings_tenant ON user_settings;
ALTER TABLE user_settings DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS expenses_tenant ON expenses;
ALTER TABLE expenses DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS income_tenant ON income;
ALTER TABLE income DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS savings_tenant ON savings;
ALTER TABLE savings DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS income_pay_schedules_tenant ON income_pay_schedules;
ALTER TABLE income_pay_schedules DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS budgets_tenant ON budgets;
ALTER TABLE budgets DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS planned_expenses_tenant ON planned_expenses;
ALTER TABLE planned_expenses DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS recurring_expenses_tenant ON recurring_expenses;
ALTER TABLE recurring_expenses DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tags_tenant ON tags;
ALTER TABLE tags DISABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS users_tenant ON users;
ALTER TABLE users DISABLE ROW LEVEL SECURITY;

DROP FUNCTION IF EXISTS app_rls_allowed_user_row(UUID);
DROP FUNCTION IF EXISTS app_rls_allowed(UUID);
