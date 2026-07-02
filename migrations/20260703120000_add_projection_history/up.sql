-- Projection history: one frozen row per closed pay period, tied to the pay schedule it was
-- computed under. Past periods never change, so freezing their aggregates (income, planned
-- spent, free, cumulative) avoids recomputing them on every request. The current period is
-- computed live seeded from the latest frozen row; future periods stay live-projected.
--
-- Values are stored in the display currency they were converted to at freeze time (a closed
-- period is a historical fact). `currency` records that denomination so a later display-currency
-- change can invalidate and re-initialize the frozen rows.

CREATE TABLE projection_history (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    schedule_id   UUID NOT NULL REFERENCES income_pay_schedules(id) ON DELETE CASCADE,
    pay_date      DATE NOT NULL,
    start_date    DATE NOT NULL,
    end_date      DATE NOT NULL,
    income        INT4 NOT NULL,
    planned_spent INT4 NOT NULL,
    free          INT4 NOT NULL,
    cumulative    INT4 NOT NULL,
    currency      currency_code NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, schedule_id, pay_date)
);

CREATE INDEX projection_history_user_id_idx ON projection_history (user_id);
CREATE INDEX projection_history_user_schedule_idx
    ON projection_history (user_id, schedule_id, pay_date);

-- Row-level security, mirroring the other user-owned tables (see 20250612130000_row_level_security).
ALTER TABLE projection_history ENABLE ROW LEVEL SECURITY;
CREATE POLICY projection_history_tenant ON projection_history
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));
