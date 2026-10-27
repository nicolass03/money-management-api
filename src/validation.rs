use chrono::NaiveDate;

use crate::error::ApiError;
use crate::models::{CurrencyCode, InterestRateType, PayFrequency};

pub const MAX_NAME_LEN: usize = 200;
pub const MAX_TAG_LEN: usize = 50;
pub const MAX_TAGS: usize = 20;
pub const MAX_AMOUNT: i32 = 1_000_000_000;

pub fn parse_date(value: &str) -> Result<NaiveDate, ApiError> {
    if !regex_like_date(value) {
        return Err(ApiError::BadRequest("invalid date".into()));
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest("invalid date".into()))
}

pub fn regex_like_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.chars().all(|c| c.is_ascii_digit() || c == '-')
}

pub fn parse_currency(value: &str) -> Result<CurrencyCode, ApiError> {
    match value.to_lowercase().as_str() {
        "eur" => Ok(CurrencyCode::Eur),
        "usd" => Ok(CurrencyCode::Usd),
        "cop" => Ok(CurrencyCode::Cop),
        _ => Err(ApiError::BadRequest("invalid currency".into())),
    }
}

pub fn parse_language(value: &str) -> Result<String, ApiError> {
    match value.to_lowercase().as_str() {
        "en" => Ok("en".to_string()),
        "es" => Ok("es".to_string()),
        _ => Err(ApiError::BadRequest("invalid language".into())),
    }
}

/// Themes are defined entirely on the frontend, so the API stores the code verbatim rather than
/// validating against a fixed set (which would require an API deploy per new theme). We only
/// guard the shape: a short, non-empty slug of `[A-Za-z0-9_-]`.
pub fn parse_theme(value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(trimmed.to_string())
    } else {
        Err(ApiError::BadRequest("invalid theme".into()))
    }
}

pub fn parse_pay_frequency(value: &str) -> Result<PayFrequency, ApiError> {
    match value.to_lowercase().as_str() {
        "weekly" => Ok(PayFrequency::Weekly),
        "biweekly" => Ok(PayFrequency::Biweekly),
        "monthly" => Ok(PayFrequency::Monthly),
        "yearly" => Ok(PayFrequency::Yearly),
        _ => Err(ApiError::BadRequest("invalid frequency".into())),
    }
}

pub fn parse_interest_rate_type(value: &str) -> Result<InterestRateType, ApiError> {
    match value.to_lowercase().as_str() {
        "apy" => Ok(InterestRateType::Apy),
        "tae" => Ok(InterestRateType::Tae),
        "tin" => Ok(InterestRateType::Tin),
        "ea" => Ok(InterestRateType::Ea),
        "namv" => Ok(InterestRateType::Namv),
        "em" => Ok(InterestRateType::Em),
        _ => Err(ApiError::BadRequest("invalid interest rate type".into())),
    }
}

/// Interest rate in basis points: 0 (no interest) through 10,000 (100%).
pub fn require_interest_rate(bps: i32) -> Result<i32, ApiError> {
    if !(0..=10_000).contains(&bps) {
        return Err(ApiError::BadRequest("invalid interest rate".into()));
    }
    Ok(bps)
}

pub fn require_non_empty_name(name: &str) -> Result<String, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(ApiError::BadRequest("name is too long".into()));
    }
    Ok(trimmed.to_string())
}

pub fn require_positive_amount(amount: i32) -> Result<i32, ApiError> {
    if amount <= 0 || amount > MAX_AMOUNT {
        return Err(ApiError::BadRequest("invalid amount".into()));
    }
    Ok(amount)
}

/// An amount that may be zero (e.g. voiding a recurring charge that didn't happen).
pub fn require_non_negative_amount(amount: i32) -> Result<i32, ApiError> {
    if !(0..=MAX_AMOUNT).contains(&amount) {
        return Err(ApiError::BadRequest("invalid amount".into()));
    }
    Ok(amount)
}

/// An account's starting balance. Non-negative (an account can later go negative via charges,
/// but you can't open one already in the red) and bounded like other money values.
pub fn require_initial_amount(amount: i32) -> Result<i32, ApiError> {
    if !(0..=MAX_AMOUNT).contains(&amount) {
        return Err(ApiError::BadRequest("invalid initial amount".into()));
    }
    Ok(amount)
}

/// Optional account name: trimmed; `None`/blank means an unnamed account.
pub fn parse_optional_name(name: Option<&str>) -> Result<Option<String>, ApiError> {
    match name.map(str::trim) {
        Some(trimmed) if !trimmed.is_empty() => {
            if trimmed.chars().count() > MAX_NAME_LEN {
                return Err(ApiError::BadRequest("name is too long".into()));
            }
            Ok(Some(trimmed.to_string()))
        }
        _ => Ok(None),
    }
}

/// Validates an extra-spent limit. The limit is an optional positive amount (in display-currency
/// minor units); `None` clears it. Zero is rejected so an unset limit and a zero limit can't be
/// confused — clearing the limit is expressed by `null`, not `0`.
pub fn require_extra_spent_limit(amount: i32) -> Result<i32, ApiError> {
    if amount <= 0 || amount > MAX_AMOUNT {
        return Err(ApiError::BadRequest("invalid extra spent limit".into()));
    }
    Ok(amount)
}

pub fn parse_tag_names(tags: &[String]) -> Result<Vec<String>, ApiError> {
    if tags.len() > MAX_TAGS {
        return Err(ApiError::BadRequest("too many tags".into()));
    }

    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let name = tag.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        if name.chars().count() > MAX_TAG_LEN {
            return Err(ApiError::BadRequest("tag is too long".into()));
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        result.push(name);
    }
    if result.is_empty() {
        return Err(ApiError::BadRequest("at least one tag is required".into()));
    }
    Ok(result)
}

pub fn today_utc() -> NaiveDate {
    chrono::Utc::now().date_naive()
}

/// Reference calendar date for pay-period boundaries and day-relative views.
/// Clients should pass their local `asOf`; server falls back to UTC today.
pub fn resolve_reference_date(as_of: Option<&str>) -> Result<NaiveDate, ApiError> {
    match as_of {
        Some(value) if !value.is_empty() => parse_date(value),
        _ => Ok(today_utc()),
    }
}

/// The client's local "today" (`asOf`) for date rules on writes — "must be in the future", "paid
/// today", "within the current pay period" — so they follow the user's calendar instead of UTC's.
/// Every timezone's date is within a day of UTC's, so anything further off is rejected. Falls back
/// to UTC today when omitted.
pub fn resolve_client_today(as_of: Option<&str>) -> Result<NaiveDate, ApiError> {
    let today = today_utc();
    let Some(value) = as_of.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(today);
    };
    let as_of = parse_date(value).map_err(|_| ApiError::BadRequest("invalid as-of date".into()))?;
    if (as_of - today).num_days().abs() > 1 {
        return Err(ApiError::BadRequest("invalid as-of date".into()));
    }
    Ok(as_of)
}
