-- Transfers move money between two of the user's own accounts. They live in their own tables (not
-- as an expense + income pair) so they never count as spend or income in period views, reports or
-- projections: a same-currency transfer is net-zero across accounts, so only account balances
-- (services::accounts::compute_balances) see them. v1 supports same-currency accounts only; the
-- API enforces that both accounts share the row's currency.
--
-- Account FKs use the default NO ACTION (not CASCADE): accounts are archived, never hard-deleted,
-- and a cascade would silently drop one side of a transfer and corrupt the other account's
-- balance. Deleting a user still cascades, since NO ACTION is checked at the end of the statement.

-- A recurring transfer template; the daily job materializes one `transfers` row per occurrence.
CREATE TABLE recurring_transfers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT,
    from_account_id UUID NOT NULL REFERENCES accounts(id),
    to_account_id UUID NOT NULL REFERENCES accounts(id),
    amount INT4 NOT NULL CHECK (amount > 0),
    currency currency_code NOT NULL,
    anchor_date DATE NOT NULL,
    frequency pay_frequency NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    CHECK (from_account_id <> to_account_id)
);

CREATE INDEX recurring_transfers_user_id_idx ON recurring_transfers (user_id);

-- Deletes are always soft (`deleted_at`): a tombstoned occurrence keeps its
-- (recurring_transfer_id, date) slot, so the daily job's catch-up never re-creates it.
CREATE TABLE transfers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT,
    from_account_id UUID NOT NULL REFERENCES accounts(id),
    to_account_id UUID NOT NULL REFERENCES accounts(id),
    amount INT4 NOT NULL CHECK (amount > 0),
    currency currency_code NOT NULL,
    date DATE NOT NULL,
    recurring_transfer_id UUID REFERENCES recurring_transfers(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    CHECK (from_account_id <> to_account_id)
);

CREATE INDEX transfers_user_date_idx ON transfers (user_id, date);
CREATE UNIQUE INDEX transfers_recurring_date_unique
    ON transfers (recurring_transfer_id, date) WHERE recurring_transfer_id IS NOT NULL;

-- Row-level security, mirroring the other user-owned tables. Data grants for app_api come from the
-- default privileges set in 20261027140000_app_api_role.
ALTER TABLE recurring_transfers ENABLE ROW LEVEL SECURITY;
CREATE POLICY recurring_transfers_tenant ON recurring_transfers
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));

ALTER TABLE transfers ENABLE ROW LEVEL SECURITY;
CREATE POLICY transfers_tenant ON transfers
    USING (app_rls_allowed(user_id))
    WITH CHECK (app_rls_allowed(user_id));
