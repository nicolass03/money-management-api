use std::collections::HashSet;

use chrono::NaiveDate;
use uuid::Uuid;

use crate::error::ApiError;
use crate::repos::transfers::{self as transfers_repo, TransferFields};
use crate::repos::{accounts as accounts_repo, connection, recurring_transfers, settings as settings_repo};
use crate::services::pay_periods::{occurs_on, schedule_from_recurring_transfer};
use crate::state::DbPool;

/// Materializes the user's recurring transfers due on each of `dates` (mirrors
/// `charge_due_income`). Idempotent: the `(recurring_transfer_id, date)` unique index — which
/// soft-deleted tombstones keep occupied — turns a repeat insert into a no-op. A template whose
/// account was archived is paused, so money never moves into or out of an account that no longer
/// shows a balance. Returns the number of transfers created.
pub async fn charge_due_transfers(
    pool: &DbPool,
    user_id: Uuid,
    dates: &[NaiveDate],
) -> Result<i32, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let templates = recurring_transfers::list_all_with_conn(&mut conn, user_id).await?;
    if templates.is_empty() {
        return Ok(0);
    }
    let active_accounts: HashSet<Uuid> = accounts_repo::list_active_with_conn(&mut conn, user_id)
        .await?
        .into_iter()
        .map(|account| account.id)
        .collect();
    let mut created = 0;

    for &due_date in dates {
        for template in &templates {
            // Like recurring expenses: no occurrence before the template was last saved, so
            // catching up on missed days never backfills history.
            if due_date < template.updated_at.date_naive()
                || !active_accounts.contains(&template.from_account_id)
                || !active_accounts.contains(&template.to_account_id)
                || !occurs_on(&schedule_from_recurring_transfer(template), due_date)
            {
                continue;
            }

            let fields = TransferFields {
                name: template.name.clone(),
                from_account_id: template.from_account_id,
                to_account_id: template.to_account_id,
                amount: template.amount,
                currency: template.currency,
                date: due_date,
            };
            if transfers_repo::insert_scheduled(&mut conn, user_id, template.id, &fields).await? {
                created += 1;
            }
        }
    }

    if created > 0 {
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
