-- "Cancel subscription" reminders: flag a subscription for cancellation and remind the user
-- 5 and 2 days before its next charge so they can cancel with the provider in time.

ALTER TABLE recurring_expenses
    ADD COLUMN cancel_reminder_enabled BOOLEAN NOT NULL DEFAULT false;

-- One row per (subscription, upcoming charge, lead-time). The daily job upserts these; web reads
-- the undismissed ones to render banners. iOS schedules its own local notifications from the flag.
CREATE TABLE subscription_reminders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recurring_expense_id UUID NOT NULL REFERENCES recurring_expenses(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    charge_date DATE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    dismissed_at TIMESTAMPTZ,
    UNIQUE (recurring_expense_id, charge_date, kind)
);

-- Drives the active-banner query: undismissed reminders for a user, ordered by charge date.
CREATE INDEX subscription_reminders_active_idx
    ON subscription_reminders (user_id, charge_date)
    WHERE dismissed_at IS NULL;

-- Row-level security, mirroring the other user-owned tables (see 20250612130000_row_level_security).
ALTER TABLE subscription_reminders ENABLE ROW LEVEL SECURITY;
CREATE POLICY subscription_reminders_tenant ON subscription_reminders
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));
