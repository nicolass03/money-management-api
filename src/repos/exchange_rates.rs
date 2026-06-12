use chrono::Utc;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::error::ApiError;
use crate::models::{CurrencyCode, ExchangeRateSnapshotRow};
use crate::schema::exchange_rate_snapshots;
use crate::services::currency::ExchangeRates;
use diesel_async::AsyncPgConnection;

pub async fn get_latest_snapshot(
    conn: &mut AsyncPgConnection,
) -> Result<Option<ExchangeRateSnapshotRow>, ApiError> {
    exchange_rate_snapshots::table
        .order(exchange_rate_snapshots::fetched_at.desc())
        .select(ExchangeRateSnapshotRow::as_select())
        .first(conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn save_snapshot(conn: &mut AsyncPgConnection, rates: &ExchangeRates) -> Result<(), ApiError> {
    let base = match rates.base.to_lowercase().as_str() {
        "eur" => CurrencyCode::Eur,
        "usd" => CurrencyCode::Usd,
        "cop" => CurrencyCode::Cop,
        _ => CurrencyCode::Usd,
    };
    let fetched_at = chrono::DateTime::parse_from_rfc3339(&rates.fetched_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    diesel::insert_into(exchange_rate_snapshots::table)
        .values((
            exchange_rate_snapshots::base_currency.eq(base),
            exchange_rate_snapshots::rates_json.eq(serde_json::to_value(&rates.rates).unwrap()),
            exchange_rate_snapshots::fetched_at.eq(fetched_at),
        ))
        .execute(conn)
        .await?;
    Ok(())
}
