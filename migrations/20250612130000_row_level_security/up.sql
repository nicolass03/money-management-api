-- Row-level security: defense in depth alongside application user_id filters.
-- Session vars: app.user_id (tenant UUID), app.is_admin (internal jobs).

CREATE OR REPLACE FUNCTION app_rls_allowed(owned_user_id UUID) RETURNS BOOLEAN AS $$
BEGIN
    IF COALESCE(current_setting('app.is_admin', true), '') = 'true' THEN
        RETURN TRUE;
    END IF;
    RETURN owned_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid;
END;
$$ LANGUAGE plpgsql STABLE;

CREATE OR REPLACE FUNCTION app_rls_allowed_user_row(row_user_id UUID) RETURNS BOOLEAN AS $$
    SELECT app_rls_allowed(row_user_id);
$$ LANGUAGE sql STABLE;

-- users: row key is id
ALTER TABLE users ENABLE ROW LEVEL SECURITY;
CREATE POLICY users_tenant ON users
    USING (app_rls_allowed(id))
    WITH CHECK (app_rls_allowed(id));

-- user-owned entity tables
ALTER TABLE tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY tags_tenant ON tags
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE recurring_expenses ENABLE ROW LEVEL SECURITY;
CREATE POLICY recurring_expenses_tenant ON recurring_expenses
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE planned_expenses ENABLE ROW LEVEL SECURITY;
CREATE POLICY planned_expenses_tenant ON planned_expenses
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE budgets ENABLE ROW LEVEL SECURITY;
CREATE POLICY budgets_tenant ON budgets
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE income_pay_schedules ENABLE ROW LEVEL SECURITY;
CREATE POLICY income_pay_schedules_tenant ON income_pay_schedules
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE savings ENABLE ROW LEVEL SECURITY;
CREATE POLICY savings_tenant ON savings
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE income ENABLE ROW LEVEL SECURITY;
CREATE POLICY income_tenant ON income
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE expenses ENABLE ROW LEVEL SECURITY;
CREATE POLICY expenses_tenant ON expenses
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE user_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_settings_tenant ON user_settings
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

-- Junction tables (no user_id): allow via parent ownership
ALTER TABLE budget_tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY budget_tags_tenant ON budget_tags
    USING (
        EXISTS (
            SELECT 1 FROM budgets b
            WHERE b.id = budget_tags.budget_id AND app_rls_allowed(b.user_id)
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM budgets b
            WHERE b.id = budget_tags.budget_id AND app_rls_allowed(b.user_id)
        )
    );

ALTER TABLE expense_tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY expense_tags_tenant ON expense_tags
    USING (
        EXISTS (
            SELECT 1 FROM expenses e
            WHERE e.id = expense_tags.expense_id AND app_rls_allowed(e.user_id)
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM expenses e
            WHERE e.id = expense_tags.expense_id AND app_rls_allowed(e.user_id)
        )
    );

ALTER TABLE planned_expense_tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY planned_expense_tags_tenant ON planned_expense_tags
    USING (
        EXISTS (
            SELECT 1 FROM planned_expenses p
            WHERE p.id = planned_expense_tags.planned_expense_id AND app_rls_allowed(p.user_id)
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM planned_expenses p
            WHERE p.id = planned_expense_tags.planned_expense_id AND app_rls_allowed(p.user_id)
        )
    );

ALTER TABLE recurring_expense_tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY recurring_expense_tags_tenant ON recurring_expense_tags
    USING (
        EXISTS (
            SELECT 1 FROM recurring_expenses r
            WHERE r.id = recurring_expense_tags.recurring_expense_id AND app_rls_allowed(r.user_id)
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM recurring_expenses r
            WHERE r.id = recurring_expense_tags.recurring_expense_id AND app_rls_allowed(r.user_id)
        )
    );

-- Global FX cache (shared across tenants)
ALTER TABLE exchange_rate_snapshots ENABLE ROW LEVEL SECURITY;
CREATE POLICY exchange_rate_snapshots_all ON exchange_rate_snapshots
    USING (true)
    WITH CHECK (true);
