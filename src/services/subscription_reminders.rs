use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::{connection, recurring_expenses, subscription_reminders};
use crate::services::pay_periods::{add_days, get_next_pay_date, schedule_from_recurring};
use crate::state::DbPool;

const FIVE_DAY_KIND: &str = "five_day";
const TWO_DAY_KIND: &str = "two_day";

/// Records cancellation reminders due on `date` (an ISO `YYYY-MM-DD` string). For every subscription
/// the user flagged via `cancel_reminder_enabled`, this checks whether `date` is exactly 5 or 2 days
/// before its next charge and, if so, upserts the matching reminder row. Idempotent across runs, so
/// re-running for the same day creates nothing new. Returns the count of newly created rows.
pub async fn generate_subscription_reminders_for_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
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
        let next_charge = get_next_pay_date(&schedule, date);

        // Respect an end date: a charge past the last payment date never happens, so don't remind.
        if let Some(last) = recurring.last_payment_date {
            if next_charge.as_str() > last.format("%Y-%m-%d").to_string().as_str() {
                continue;
            }
        }

        let Ok(charge_date) = chrono::NaiveDate::parse_from_str(&next_charge, "%Y-%m-%d") else {
            continue;
        };

        for (offset, kind) in [(5, FIVE_DAY_KIND), (2, TWO_DAY_KIND)] {
            if add_days(&next_charge, -offset) == date
                && subscription_reminders::upsert(&mut conn, user_id, recurring.id, kind, charge_date)
                    .await?
            {
                created += 1;
            }
        }
    }
    Ok(created)
}
