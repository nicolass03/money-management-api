DROP INDEX IF EXISTS expenses_budget_id_idx;
CREATE INDEX tags_user_id_idx ON tags (user_id);
CREATE INDEX projection_history_user_id_idx ON projection_history (user_id);
CREATE INDEX projection_history_user_schedule_idx
    ON projection_history (user_id, schedule_id, pay_date);
