-- One-time expenses can be undated (e.g. a debt with no due date). Undated items stay out of
-- projections and are recorded only when the user pays them.
ALTER TABLE planned_expenses ALTER COLUMN date DROP NOT NULL;
