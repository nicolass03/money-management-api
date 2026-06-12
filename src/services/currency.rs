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
        return amount_minor;
    };

    let major_in_usd = f64::from(amount_minor) / minor_divisor(from) / from_rate;
    let major_in_target = major_in_usd * to_rate;
    (major_in_target * minor_divisor(to)).round() as i32
}

fn to_iso_key(currency: CurrencyCode) -> String {
    currency.to_iso().to_string()
}
