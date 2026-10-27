DROP INDEX IF EXISTS income_interest_account_date_unique;
DELETE FROM income WHERE source = 'interest';
ALTER TABLE accounts
  DROP CONSTRAINT IF EXISTS accounts_interest_cursor_check,
  DROP COLUMN IF EXISTS interest_carry_micros,
  DROP COLUMN IF EXISTS interest_accrued_through,
  DROP COLUMN IF EXISTS interest_rate_type,
  DROP COLUMN IF EXISTS interest_rate_bps;
DROP TYPE IF EXISTS interest_rate_type;
