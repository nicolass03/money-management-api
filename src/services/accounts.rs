use std::collections::HashMap;

use chrono::NaiveDate;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{AccountRow, CurrencyCode};
use crate::schema::{expenses, income, transfers, user_settings};
use crate::services::currency::{to_base_major, ExchangeRates};

/// Derived current balance per account, in the account's own currency:
/// `initial_amount + Σ(income assigned) − Σ(expenses assigned) + Σ(transfers in) − Σ(transfers
/// out)`, counting only rows dated within
/// `[projection_start_date, as_of]`. `initial_amount` is therefore the account's balance on the
/// user's projection start date — the same meaning the projection seed gives it — so moving the
/// start date to today and entering current balances "starts over" without touching history.
/// With no start date set, all history up to `as_of` counts. No currency conversion is needed:
/// every row written with an `account_id` carries that account's currency (the forms force
/// currency-follows-account), and the recurring charge job stores its charge in the chosen
/// account's currency. Transfers are same-currency (v1), so both sides share the row's currency.
pub async fn compute_balances(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    accounts_list: &[AccountRow],
    as_of: NaiveDate,
) -> Result<HashMap<Uuid, i64>, ApiError> {
    let start_date: Option<NaiveDate> = user_settings::table
        .filter(user_settings::user_id.eq(user_id))
        .select(user_settings::projection_start_date)
        .first(conn)
        .await
        .optional()?
        .flatten();

    // Aggregate the per-account sums in Postgres (GROUP BY) rather than loading every expense/income
    // row into memory and folding here — the DB returns one row per account instead of one per
    // transaction. `sum` over Int4 yields a nullable BigInt (`Option<i64>`).
    let mut expense_query = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::account_id.is_not_null())
        .filter(expenses::date.le(as_of))
        .group_by(expenses::account_id)
        .select((expenses::account_id, diesel::dsl::sum(expenses::amount)))
        .into_boxed();
    if let Some(start) = start_date {
        expense_query = expense_query.filter(expenses::date.ge(start));
    }
    let expense_sums: Vec<(Option<Uuid>, Option<i64>)> = expense_query.load(conn).await?;

    let mut income_query = income::table
        .filter(income::user_id.eq(user_id))
        .filter(income::account_id.is_not_null())
        .filter(income::deleted_at.is_null())
        .filter(income::date.le(as_of))
        .group_by(income::account_id)
        .select((income::account_id, diesel::dsl::sum(income::amount)))
        .into_boxed();
    if let Some(start) = start_date {
        income_query = income_query.filter(income::date.ge(start));
    }
    let income_sums: Vec<(Option<Uuid>, Option<i64>)> = income_query.load(conn).await?;

    // Transfers between the user's own accounts: out of the source, into the destination.
    let mut transfers_out_query = transfers::table
        .filter(transfers::user_id.eq(user_id))
        .filter(transfers::deleted_at.is_null())
        .filter(transfers::date.le(as_of))
        .group_by(transfers::from_account_id)
        .select((transfers::from_account_id, diesel::dsl::sum(transfers::amount)))
        .into_boxed();
    let mut transfers_in_query = transfers::table
        .filter(transfers::user_id.eq(user_id))
        .filter(transfers::deleted_at.is_null())
        .filter(transfers::date.le(as_of))
        .group_by(transfers::to_account_id)
        .select((transfers::to_account_id, diesel::dsl::sum(transfers::amount)))
        .into_boxed();
    if let Some(start) = start_date {
        transfers_out_query = transfers_out_query.filter(transfers::date.ge(start));
        transfers_in_query = transfers_in_query.filter(transfers::date.ge(start));
    }
    let transfers_out: Vec<(Uuid, Option<i64>)> = transfers_out_query.load(conn).await?;
    let transfers_in: Vec<(Uuid, Option<i64>)> = transfers_in_query.load(conn).await?;

    // Fold every signed per-account sum into one delta map.
    let mut deltas: HashMap<Uuid, i64> = HashMap::new();
    let assigned = |rows: Vec<(Option<Uuid>, Option<i64>)>, sign: i64| {
        rows.into_iter()
            .filter_map(move |(id, total)| Some((id?, sign * total.unwrap_or(0))))
    };
    let transferred = |rows: Vec<(Uuid, Option<i64>)>, sign: i64| {
        rows.into_iter()
            .map(move |(id, total)| (id, sign * total.unwrap_or(0)))
    };
    for (id, delta) in assigned(income_sums, 1)
        .chain(assigned(expense_sums, -1))
        .chain(transferred(transfers_in, 1))
        .chain(transferred(transfers_out, -1))
    {
        *deltas.entry(id).or_default() += delta;
    }

    Ok(accounts_list
        .iter()
        .map(|account| {
            let balance = i64::from(account.initial_amount)
                + deltas.get(&account.id).copied().unwrap_or(0);
            (account.id, balance)
        })
        .collect())
}

/// Picks the account a same-currency charge should draw from: among non-archived accounts whose
/// currency matches `currency` and whose balance covers `min_amount`, the one with the highest
/// balance. Returns `None` when no matching-currency account can cover the charge.
pub fn pick_funded_account(
    accounts_list: &[AccountRow],
    balances: &HashMap<Uuid, i64>,
    currency: CurrencyCode,
    min_amount: i32,
) -> Option<Uuid> {
    accounts_list
        .iter()
        .filter(|a| a.currency == currency)
        .filter_map(|a| balances.get(&a.id).map(|b| (a.id, *b)))
        .filter(|(_, balance)| *balance >= i64::from(min_amount))
        .max_by_key(|(_, balance)| *balance)
        .map(|(id, _)| id)
}

/// Fallback account when no matching-currency account has enough funds: the highest-balance
/// account in `currency` (typically the display currency), regardless of whether it can cover the
/// charge — the charge is allowed to drive it negative.
pub fn pick_richest_account(
    accounts_list: &[AccountRow],
    balances: &HashMap<Uuid, i64>,
    currency: CurrencyCode,
) -> Option<Uuid> {
    accounts_list
        .iter()
        .filter(|a| a.currency == currency)
        .filter_map(|a| balances.get(&a.id).map(|b| (a.id, *b)))
        .max_by_key(|(_, balance)| *balance)
        .map(|(id, _)| id)
}

/// Last-resort fallback for a recurring charge: the highest-balance account in *any* currency,
/// comparing balances by value (converted with `rates`), not raw minor units — COP has no minor
/// unit, so its raw numbers dwarf the others. Used only when the user has no account in the display
/// currency, so that a charge is never stranded off-book (`account_id = NULL`, invisible to every
/// balance). The caller converts the amount into the chosen account's currency.
pub fn pick_richest_any_currency(
    accounts_list: &[AccountRow],
    balances: &HashMap<Uuid, i64>,
    rates: &ExchangeRates,
) -> Option<AccountRow> {
    accounts_list
        .iter()
        .filter_map(|a| {
            let balance = *balances.get(&a.id)?;
            Some((a, to_base_major(balance, a.currency, rates)?))
        })
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(a, _)| a.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn rates() -> ExchangeRates {
        ExchangeRates {
            base: "USD".into(),
            rates: HashMap::from([
                ("USD".to_string(), 1.0),
                ("EUR".to_string(), 0.9),
                ("COP".to_string(), 4_000.0),
            ]),
            fetched_at: Utc::now().to_rfc3339(),
        }
    }

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

    #[test]
    fn any_currency_pick_is_used_when_no_display_currency_account_exists() {
        // Display currency is USD but the user holds only EUR/COP accounts: rather than strand the
        // charge off-book, the richest account in any currency is chosen — by value: €30.00 beats
        // COP 50,000 (~$12.50) even though 50,000 > 3,000 in raw minor units.
        let eur = account(CurrencyCode::Eur);
        let cop = account(CurrencyCode::Cop);
        let accounts = vec![eur.clone(), cop.clone()];
        let balances = HashMap::from([(eur.id, 3_000), (cop.id, 50_000)]);

        assert_eq!(pick_richest_account(&accounts, &balances, CurrencyCode::Usd), None);
        assert_eq!(
            pick_richest_any_currency(&accounts, &balances, &rates()).map(|a| a.id),
            Some(eur.id)
        );
    }

    #[test]
    fn any_currency_pick_is_none_without_accounts() {
        let accounts: Vec<AccountRow> = vec![];
        let balances = HashMap::new();
        assert!(pick_richest_any_currency(&accounts, &balances, &rates()).is_none());
    }
}
