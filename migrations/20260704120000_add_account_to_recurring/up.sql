-- Recurring-expense templates can optionally pin an account. When set, the daily charge job draws
-- from that account (and the form forces currency-follows-account, like every other money form).
-- When NULL (the default), the charge job keeps its by-currency selection: it picks a funded
-- same-currency account, else the richest display-currency account, else any account.
-- ON DELETE SET NULL so archiving/removing an account degrades the template back to by-currency
-- selection rather than destroying it.
ALTER TABLE recurring_expenses
    ADD COLUMN account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;

CREATE INDEX recurring_expenses_account_id_idx ON recurring_expenses (account_id);
