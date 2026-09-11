-- Soft-delete income pay schedules: stop future pay dates without erasing materialized income.
-- `income.schedule_id` keeps its NO ACTION FK, so a hard delete can never wipe income history.
ALTER TABLE income_pay_schedules
    ADD COLUMN deleted_at TIMESTAMPTZ NULL;
