-- The original check rejects interest rows, so they go first (as in 20261027180000's down).
DELETE FROM income WHERE source = 'interest';
ALTER TABLE income DROP CONSTRAINT income_scheduled_consistency_chk;
ALTER TABLE income ADD CONSTRAINT income_scheduled_consistency_chk
    CHECK (
        (source = 'scheduled' AND schedule_id IS NOT NULL)
        OR (source = 'manual' AND schedule_id IS NULL)
    );
