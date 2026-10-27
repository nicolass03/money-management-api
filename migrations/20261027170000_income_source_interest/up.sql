-- Daily interest an account earns is credited as income with its own source, so it counts in
-- balances, projections and reports but stays out of the income list. Its own migration: Postgres
-- can't use a new enum value in the transaction that adds it.
ALTER TYPE income_source ADD VALUE IF NOT EXISTS 'interest';
