UPDATE planned_expenses SET date = CURRENT_DATE WHERE date IS NULL;
ALTER TABLE planned_expenses ALTER COLUMN date SET NOT NULL;
