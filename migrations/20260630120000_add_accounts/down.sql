ALTER TABLE planned_expenses DROP COLUMN account_id;
ALTER TABLE income_pay_schedules DROP COLUMN account_id;
ALTER TABLE income DROP COLUMN account_id;
ALTER TABLE expenses DROP COLUMN account_id;

DROP TABLE IF EXISTS accounts;
