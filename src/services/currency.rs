use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::CurrencyCode;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeRates {
    pub base: String,
    pub rates: HashMap<String, f64>,
    pub fetched_at: String,
}

pub fn minor_divisor(currency: CurrencyCode) -> f64 {
    if currency == CurrencyCode::Cop {
        1.0
    } else {
        100.0
    }
}

pub fn convert_amount(
    amount_minor: i32,
    from: CurrencyCode,
    to: CurrencyCode,
    rates: &ExchangeRates,
) -> i32 {
    if from == to {
        return amount_minor;
    }

    let (Some(major_in_base), Some(to_rate)) = (
        to_base_major(i64::from(amount_minor), from, rates),
        rate(to, rates),
    ) else {
        // Unreachable in practice: rates are only accepted when they cover every supported
        // currency (`has_all_currencies`). Returning the amount unconverted would silently mix
        // currencies, so make it loud if it ever happens.
        tracing::error!(?from, ?to, "missing exchange rate; amount left unconverted");
        return amount_minor;
    };

    (major_in_base * to_rate * minor_divisor(to)).round() as i32
}

/// `amount_minor` in major units of the rates' base currency, so amounts in different currencies
/// can be compared. `None` when the currency's rate is missing.
pub fn to_base_major(amount_minor: i64, currency: CurrencyCode, rates: &ExchangeRates) -> Option<f64> {
    rate(currency, rates).map(|rate| amount_minor as f64 / minor_divisor(currency) / rate)
}

fn rate(currency: CurrencyCode, rates: &ExchangeRates) -> Option<f64> {
    rates.rates.get(currency.to_iso()).copied()
}

/// Whether `rates` can convert between every supported currency.
pub fn has_all_currencies(rates: &HashMap<String, f64>) -> bool {
    [CurrencyCode::Eur, CurrencyCode::Usd, CurrencyCode::Cop]
        .into_iter()
        .all(|currency| rates.get(currency.to_iso()).is_some_and(|rate| *rate > 0.0))
}
