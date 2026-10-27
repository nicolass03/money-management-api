-- Accounts that pay interest daily (savings accounts like Nu "cajitas" or Revolut savings). The
-- daily job credits each day's interest as an `income` row with source 'interest'.
-- `interest_rate_type` is the convention the bank quotes the rate in: APY/AER, TAE and E.A. are
-- effective annual; TIN is nominal annual; N.A.M.V. is nominal annual compounded monthly; E.M. is
-- effective monthly.
CREATE TYPE interest_rate_type AS ENUM ('apy', 'tae', 'tin', 'ea', 'namv', 'em');

-- `interest_rate_bps`: basis points (8.25% = 825); 0 = no interest.
-- `interest_accrued_through`: last day credited (set to the enable date, so nothing is backfilled).
-- `interest_carry_micros`: accrued fraction of a minor unit not yet credited, in millionths — the
-- bank only pays whole cents, carrying the rest to the next day.
ALTER TABLE accounts
  ADD COLUMN interest_rate_bps INTEGER NOT NULL DEFAULT 0
    CHECK (interest_rate_bps BETWEEN 0 AND 10000),
  ADD COLUMN interest_rate_type interest_rate_type NOT NULL DEFAULT 'apy',
  ADD COLUMN interest_accrued_through DATE,
  ADD COLUMN interest_carry_micros INTEGER NOT NULL DEFAULT 0
    CHECK (interest_carry_micros BETWEEN 0 AND 999999),
  ADD CONSTRAINT accounts_interest_cursor_check
    CHECK (interest_rate_bps = 0 OR interest_accrued_through IS NOT NULL);

-- One interest credit per account and day; also serves the per-account interest list.
CREATE UNIQUE INDEX income_interest_account_date_unique
  ON income (account_id, date)
  WHERE source = 'interest';
