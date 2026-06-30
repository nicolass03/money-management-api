use std::collections::HashMap;

use chrono::NaiveDate;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{AccountRow, CurrencyCode};
use crate::schema::{expenses, income};

/// Derived current balance per account, in the account's own currency:
/// `initial_amount + Σ(income assigned) − Σ(expenses assigned)`, counting only rows dated on or
/// before `as_of`. No currency conversion is needed: every row written with an `account_id`
/// carries that account's currency (the forms force currency-follows-account), and the recurring
/// charge job stores its charge in the chosen account's currency.
pub async fn compute_balances(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    accounts_list: &[AccountRow],
    as_of: NaiveDate,
) -> Result<HashMap<Uuid, i32>, ApiError> {
    let expense_rows: Vec<(Option<Uuid>, i32)> = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::account_id.is_not_null())
        .filter(expenses::date.le(as_of))
        .select((expenses::account_id, expenses::amount))
        .load(conn)
        .await?;

    let income_rows: Vec<(Option<Uuid>, i32)> = income::table
        .filter(income::user_id.eq(user_id))
        .filter(income::account_id.is_not_null())
        .filter(income::deleted_at.is_null())
        .filter(income::date.le(as_of))
        .select((income::account_id, income::amount))
        .load(conn)
        .await?;

    let fold = |rows: Vec<(Option<Uuid>, i32)>| -> HashMap<Uuid, i64> {
        let mut map: HashMap<Uuid, i64> = HashMap::new();
        for (id, amount) in rows {
            if let Some(id) = id {
                *map.entry(id).or_default() += amount as i64;
            }
        }
        map
    };
    let expense_map = fold(expense_rows);
    let income_map = fold(income_rows);

    Ok(accounts_list
        .iter()
        .map(|account| {
            let balance = account.initial_amount as i64
                + income_map.get(&account.id).copied().unwrap_or(0)
                - expense_map.get(&account.id).copied().unwrap_or(0);
            (account.id, balance.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        })
        .collect())
}

/// Picks the account a same-currency charge should draw from: among non-archived accounts whose
/// currency matches `currency` and whose balance covers `min_amount`, the one with the highest
/// balance. Returns `None` when no matching-currency account can cover the charge.
pub fn pick_funded_account(
    accounts_list: &[AccountRow],
    balances: &HashMap<Uuid, i32>,
    currency: CurrencyCode,
    min_amount: i32,
) -> Option<Uuid> {
    accounts_list
        .iter()
        .filter(|a| a.currency == currency)
        .filter_map(|a| balances.get(&a.id).map(|b| (a.id, *b)))
        .filter(|(_, balance)| *balance >= min_amount)
        .max_by_key(|(_, balance)| *balance)
        .map(|(id, _)| id)
}

/// Fallback account when no matching-currency account has enough funds: the highest-balance
/// account in `currency` (typically the display currency), regardless of whether it can cover the
/// charge — the charge is allowed to drive it negative.
pub fn pick_richest_account(
    accounts_list: &[AccountRow],
    balances: &HashMap<Uuid, i32>,
    currency: CurrencyCode,
) -> Option<Uuid> {
    accounts_list
        .iter()
        .filter(|a| a.currency == currency)
        .filter_map(|a| balances.get(&a.id).map(|b| (a.id, *b)))
        .max_by_key(|(_, balance)| *balance)
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn account(currency: CurrencyCode) -> AccountRow {
        AccountRow {
            id: Uuid::new_v4(),
            _user_id: Uuid::new_v4(),
            name: None,
            currency,
            initial_amount: 0,
            archived_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn funded_pick_prefers_matching_currency_with_highest_balance() {
        let usd_a = account(CurrencyCode::Usd);
        let usd_b = account(CurrencyCode::Usd);
        let eur = account(CurrencyCode::Eur);
        let accounts = vec![usd_a.clone(), usd_b.clone(), eur.clone()];
        let balances = HashMap::from([(usd_a.id, 5_000), (usd_b.id, 20_000), (eur.id, 99_999)]);

        // Both USD accounts cover 4000; the richer USD account wins (EUR is ignored).
        assert_eq!(
            pick_funded_account(&accounts, &balances, CurrencyCode::Usd, 4_000),
            Some(usd_b.id)
        );
    }

    #[test]
    fn funded_pick_is_none_when_no_matching_account_can_cover() {
        let usd = account(CurrencyCode::Usd);
        let accounts = vec![usd.clone()];
        let balances = HashMap::from([(usd.id, 1_000)]);

        // Charge exceeds the only USD account's balance -> caller must fall back.
        assert_eq!(
            pick_funded_account(&accounts, &balances, CurrencyCode::Usd, 5_000),
            None
        );
        // Fallback to the richest display-currency account regardless of coverage.
        assert_eq!(
            pick_richest_account(&accounts, &balances, CurrencyCode::Usd),
            Some(usd.id)
        );
    }
}
