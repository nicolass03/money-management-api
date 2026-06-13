ALTER TABLE user_settings
  ADD COLUMN extra_expense_limit INTEGER NULL,
  ADD COLUMN extra_expense_limit_currency currency_code NULL;
