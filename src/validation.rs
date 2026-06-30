use chrono::NaiveDate;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PayFrequency};

pub const MAX_NAME_LEN: usize = 200;
pub const MAX_TAG_LEN: usize = 50;
pub const MAX_TAGS: usize = 20;
pub const MAX_AMOUNT: i32 = 1_000_000_000;
pub const MIN_PROJECTION_FREE_MONEY: i32 = 0;
pub const MAX_PROJECTION_FREE_MONEY: i32 = 1_000_000_000;

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

pub fn require_projection_free_money(amount: i32) -> Result<i32, ApiError> {
    if !(MIN_PROJECTION_FREE_MONEY..=MAX_PROJECTION_FREE_MONEY).contains(&amount) {
        return Err(ApiError::BadRequest("invalid projection initial free money".into()));
    }
    Ok(amount)
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

pub fn today_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Reference calendar date for pay-period boundaries and day-relative views.
/// Clients should pass their local `asOf`; server falls back to UTC today.
pub fn resolve_reference_date(as_of: Option<&str>) -> Result<String, ApiError> {
    match as_of {
        Some(value) if !value.is_empty() => {
            parse_date(value)?;
            Ok(value.to_string())
        }
        _ => Ok(today_iso()),
    }
}
