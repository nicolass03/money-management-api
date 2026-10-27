use std::slice;

use chrono::{Duration, NaiveDate, Utc};
use diesel_async::AsyncConnection;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::InterestRateType;
use crate::repos::{
    accounts as accounts_repo, connection, income as income_repo, settings as settings_repo,
};
use crate::services::accounts::compute_balances;
use crate::state::DbPool;

/// Millionths of a minor unit: the precision interest accrues at before whole units are credited.
const MICROS: i64 = 1_000_000;

/// The daily rate of a rate quoted as `rate_bps` in `rate_type`. Each convention is a periodic
/// rate `p` compounded `n` times a year, so one day is `(1 + p)^(n / 365) − 1`: an effective
/// annual rate gives `(1 + r)^(1/365) − 1`, a nominal annual one paid daily gives `r / 365`.
pub fn daily_rate(rate_bps: i32, rate_type: InterestRateType) -> f64 {
    let rate = f64::from(rate_bps) / 10_000.0;
    let (periodic, periods_per_year) = match rate_type {
        InterestRateType::Apy | InterestRateType::Tae | InterestRateType::Ea => (rate, 1.0),
        InterestRateType::Tin => (rate / 365.0, 365.0),
        InterestRateType::Namv => (rate / 12.0, 12.0),
        InterestRateType::Em => (rate, 12.0),
    };
    (1.0 + periodic).powf(periods_per_year / 365.0) - 1.0
}

/// One day's interest on `balance` plus the carried fraction, split into whole minor units to
/// credit and the fraction (in millionths) to carry to the next day. A balance at or below zero
/// earns nothing.
fn split_interest(balance: i64, daily_rate: f64, carry_micros: i32) -> (i64, i32) {
    let accrued = (balance.max(0) as f64 * daily_rate * MICROS as f64).round() as i64
        + i64::from(carry_micros);
    (accrued / MICROS, (accrued % MICROS) as i32)
}

/// Credits the daily interest of the user's interest-bearing accounts for each of `dates` (oldest
/// first) after the account's cursor (`interest_accrued_through`). Day D pays on D−1's closing
/// balance, which already holds D−1's credit, so interest compounds daily. Like the bank, only
/// whole minor units are credited and the fraction carries over. Each day's credit and cursor move
/// commit together, so a re-run never credits a day twice. Returns the number of credits created.
pub async fn accrue_interest(
    pool: &DbPool,
    user_id: Uuid,
    dates: &[NaiveDate],
) -> Result<i32, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let accounts = accounts_repo::list_active_with_conn(&mut conn, user_id).await?;
    let now = Utc::now();
    let mut created = 0;

    for account in accounts
        .iter()
        .filter(|account| account.interest_rate_bps > 0)
    {
        let Some(mut cursor) = account.interest_accrued_through else {
            continue;
        };
        let rate = daily_rate(account.interest_rate_bps, account.interest_rate_type);
        let mut carry = account.interest_carry_micros;

        for &date in dates {
            if date <= cursor {
                continue;
            }
            let balances = compute_balances(
                &mut conn,
                user_id,
                slice::from_ref(account),
                date - Duration::days(1),
            )
            .await?;
            let balance = balances.get(&account.id).copied().unwrap_or(0);
            let (amount, next_carry) = split_interest(balance, rate, carry);
            let amount = i32::try_from(amount)
                .map_err(|_| ApiError::Internal("interest credit out of range".into()))?;

            let (account_id, currency, from) = (account.id, account.currency, cursor);
            let (advanced, inserted) = conn
                .transaction(|conn| {
                    Box::pin(async move {
                        let advanced = accounts_repo::advance_interest(
                            conn, user_id, account_id, from, date, next_carry,
                        )
                        .await?;
                        let inserted = advanced
                            && amount > 0
                            && income_repo::insert_interest(
                                conn, user_id, account_id, currency, date, amount, now,
                            )
                            .await?;
                        Ok::<_, diesel::result::Error>((advanced, inserted))
                    })
                })
                .await?;
            // The cursor moved meanwhile (interest turned off and on again): leave the account.
            if !advanced {
                break;
            }
            if inserted {
                created += 1;
            }
            cursor = date;
            carry = next_carry;
        }
    }

    if created > 0 {
        settings_repo::bump_cache_revision(&mut conn, user_id).await?;
    }

    Ok(created)
}
