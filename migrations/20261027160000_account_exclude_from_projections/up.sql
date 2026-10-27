-- Lets a user keep an account (e.g. savings) out of projections: its starting balance, income,
-- expenses and recurring templates are skipped, and transfers between it and a projected account
-- count as money in or out of the projection.
ALTER TABLE accounts ADD COLUMN exclude_from_projections BOOLEAN NOT NULL DEFAULT FALSE;
