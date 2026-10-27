use chrono::NaiveDate;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{connection, recurring_expenses, subscription_reminders};
use crate::services::pay_periods::{add_days, get_next_occurrence, schedule_from_recurring};
use crate::state::DbPool;

const FIVE_DAY_KIND: &str = "five_day";
const TWO_DAY_KIND: &str = "two_day";

/// Records cancellation reminders due on `date`. For every subscription
/// the user flagged via `cancel_reminder_enabled`, this checks whether `date` is exactly 5 or 2 days
/// before its next charge and, if so, upserts the matching reminder row. Idempotent across runs, so
/// re-running for the same day creates nothing new. Returns the count of newly created rows.
pub async fn generate_subscription_reminders_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: NaiveDate,
) -> Result<i32, ApiError> {
    let candidates: Vec<_> = recurring_expenses::list_all(pool, user_id)
        .await?
        .into_iter()
        .filter(|r| r.is_subscription && r.cancel_reminder_enabled)
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut conn = connection::user_connection(pool, user_id).await?;
    let mut created = 0;
    for recurring in candidates {
        let schedule = schedule_from_recurring(&recurring);
        let charge_date = get_next_occurrence(&schedule, date);

        // Respect an end date: a charge past the last payment date never happens, so don't remind.
        if recurring.last_payment_date.is_some_and(|last| charge_date > last) {
            continue;
        }

        for (offset, kind) in [(5, FIVE_DAY_KIND), (2, TWO_DAY_KIND)] {
            if add_days(charge_date, -offset) == date
                && subscription_reminders::upsert(&mut conn, user_id, recurring.id, kind, charge_date)
                    .await?
            {
                created += 1;
            }
        }
    }
    Ok(created)
}
