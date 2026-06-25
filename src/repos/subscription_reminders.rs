use chrono::{NaiveDate, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, SubscriptionReminderResponse};
use crate::repos::connection;
use crate::schema::{recurring_expenses, subscription_reminders};
use crate::state::DbPool;

/// Records one reminder for a subscription's upcoming charge. Idempotent: a row already existing for
/// the same (recurring_expense_id, charge_date, kind) is left untouched. Returns whether a new row
/// was inserted. Takes an existing user-scoped connection so the daily job can batch upserts.
pub async fn upsert(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    recurring_expense_id: Uuid,
    kind: &str,
    charge_date: NaiveDate,
) -> Result<bool, ApiError> {
    let inserted = diesel::insert_into(subscription_reminders::table)
        .values((
            subscription_reminders::user_id.eq(user_id),
            subscription_reminders::recurring_expense_id.eq(recurring_expense_id),
            subscription_reminders::kind.eq(kind),
            subscription_reminders::charge_date.eq(charge_date),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await
        .map_err(ApiError::from)?;
    Ok(inserted > 0)
}

/// Active (undismissed, not-yet-charged) reminders for a user, enriched with subscription display
/// fields for the banner, ordered by soonest charge.
pub async fn list_active(
    pool: &DbPool,
    user_id: Uuid,
    today: NaiveDate,
) -> Result<Vec<SubscriptionReminderResponse>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let rows = subscription_reminders::table
        .inner_join(recurring_expenses::table)
        .filter(subscription_reminders::user_id.eq(user_id))
        .filter(subscription_reminders::dismissed_at.is_null())
        .filter(subscription_reminders::charge_date.ge(today))
        .order(subscription_reminders::charge_date.asc())
        .select((
            subscription_reminders::id,
            subscription_reminders::recurring_expense_id,
            recurring_expenses::name,
            subscription_reminders::kind,
            subscription_reminders::charge_date,
            recurring_expenses::amount,
            recurring_expenses::currency,
        ))
        .load::<(Uuid, Uuid, String, String, NaiveDate, i32, CurrencyCode)>(&mut conn)
        .await
        .map_err(ApiError::from)?;

    Ok(rows
        .into_iter()
        .map(
            |(id, recurring_expense_id, name, kind, charge_date, amount, currency)| {
                SubscriptionReminderResponse {
                    id,
                    recurring_expense_id,
                    name,
                    kind,
                    charge_date,
                    amount,
                    currency,
                }
            },
        )
        .collect())
}

/// Marks a reminder dismissed so the banner stops showing. Returns whether a row was updated.
pub async fn dismiss(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<bool, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let updated = diesel::update(
        subscription_reminders::table
            .filter(subscription_reminders::user_id.eq(user_id))
            .filter(subscription_reminders::id.eq(id))
            .filter(subscription_reminders::dismissed_at.is_null()),
    )
    .set(subscription_reminders::dismissed_at.eq(Utc::now()))
    .execute(&mut conn)
    .await
    .map_err(ApiError::from)?;
    Ok(updated > 0)
}
