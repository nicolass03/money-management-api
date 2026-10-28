-- Interest credits (source 'interest', no schedule) violated the scheduled/manual-only check, so
-- the daily job's insert failed and no interest was ever credited. Allow them like manual income.
ALTER TABLE income DROP CONSTRAINT income_scheduled_consistency_chk;
ALTER TABLE income ADD CONSTRAINT income_scheduled_consistency_chk
    CHECK (
        (source = 'scheduled' AND schedule_id IS NOT NULL)
        OR (source IN ('manual', 'interest') AND schedule_id IS NULL)
    );
