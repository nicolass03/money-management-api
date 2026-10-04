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

    let from_rate = rates.rates.get(&to_iso_key(from)).copied();
    let to_rate = rates.rates.get(&to_iso_key(to)).copied();

    let (Some(from_rate), Some(to_rate)) = (from_rate, to_rate) else {
        // Unreachable in practice: rates are only accepted when they cover every supported
        // currency (`has_all_currencies`). Returning the amount unconverted would silently mix
        // currencies, so make it loud if it ever happens.
        tracing::error!(?from, ?to, "missing exchange rate; amount left unconverted");
        return amount_minor;
    };

    let major_in_usd = f64::from(amount_minor) / minor_divisor(from) / from_rate;
    let major_in_target = major_in_usd * to_rate;
    (major_in_target * minor_divisor(to)).round() as i32
}

/// Whether `rates` can convert between every supported currency.
pub fn has_all_currencies(rates: &HashMap<String, f64>) -> bool {
    [CurrencyCode::Eur, CurrencyCode::Usd, CurrencyCode::Cop]
        .into_iter()
        .all(|currency| rates.get(currency.to_iso()).is_some_and(|rate| *rate > 0.0))
}

fn to_iso_key(currency: CurrencyCode) -> String {
    currency.to_iso().to_string()
}
