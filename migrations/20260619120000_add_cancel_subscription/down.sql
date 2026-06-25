DROP TABLE IF EXISTS subscription_reminders;

ALTER TABLE recurring_expenses
    DROP COLUMN cancel_reminder_enabled;
