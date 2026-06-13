use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::ApiError;
use crate::models::ExchangeRateSnapshotRow;
use crate::repos::connection;
use crate::repos::exchange_rates as exchange_rates_repo;
use crate::services::currency::ExchangeRates;
use crate::services::fx_memory::{get_memory_rates, set_memory_rates};
use crate::state::DbPool;
use diesel_async::AsyncPgConnection;

const RATES_API_URL: &str = "https://open.er-api.com/v6/latest/USD";
const RATES_TTL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
struct ApiResponse {
    result: String,
    base_code: String,
    rates: HashMap<String, f64>,
}

pub fn is_rates_stale(fetched_at: &str) -> bool {
    let Ok(parsed) = DateTime::parse_from_rfc3339(fetched_at) else {
        return true;
    };
    let fetched_ms = parsed.timestamp_millis();
    chrono::Utc::now().timestamp_millis() - fetched_ms > RATES_TTL_MS
}

pub fn parse_snapshot_row(row: &ExchangeRateSnapshotRow) -> Option<ExchangeRates> {
    let rates = row.rates_json.as_object()?;
    let mut map = HashMap::new();
    for (key, value) in rates {
        let rate = value.as_f64()?;
        map.insert(key.clone(), rate);
    }
    Some(ExchangeRates {
        base: row.base_currency.to_iso().to_string(),
        rates: map,
        fetched_at: row.fetched_at.to_rfc3339(),
    })
}

pub async fn fetch_exchange_rates() -> Result<ExchangeRates, ApiError> {
    let response = reqwest::get(RATES_API_URL)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ApiError::Internal(format!(
            "exchange rate API returned {}",
            response.status()
        )));
    }
    let data: ApiResponse = response
        .json()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if data.result != "success" {
        return Err(ApiError::Internal(
            "exchange rate API returned invalid data".into(),
        ));
    }
    Ok(ExchangeRates {
        base: data.base_code,
        rates: data.rates,
        fetched_at: Utc::now().to_rfc3339(),
    })
}

pub async fn get_exchange_rates(
    pool: &DbPool,
    force_refresh: bool,
) -> Result<ExchangeRates, ApiError> {
    if !force_refresh {
        if let Some(cached) = get_memory_rates(false) {
            return Ok(cached);
        }
    }

    let mut conn = connection::neutral_connection(pool).await?;
    let cached = exchange_rates_repo::get_latest_snapshot(&mut conn).await?;

    if let Some(ref snapshot) = cached {
        if let Some(parsed) = parse_snapshot_row(snapshot) {
            if !force_refresh && !is_rates_stale(&parsed.fetched_at) {
                set_memory_rates(parsed.clone());
                return Ok(parsed);
            }
        }
    }

    match fetch_exchange_rates().await {
        Ok(fresh) => {
            save_exchange_rates(&mut conn, &fresh).await?;
            set_memory_rates(fresh.clone());
            Ok(fresh)
        }
        Err(error) => {
            if let Some(snapshot) = cached {
                if let Some(parsed) = parse_snapshot_row(&snapshot) {
                    set_memory_rates(parsed.clone());
                    return Ok(parsed);
                }
            }
            Err(error)
        }
    }
}

pub async fn save_exchange_rates(
    conn: &mut AsyncPgConnection,
    rates: &ExchangeRates,
) -> Result<(), ApiError> {
    exchange_rates_repo::save_snapshot(conn, rates).await
}
