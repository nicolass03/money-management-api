use chrono::NaiveDate;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PayFrequency};

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
    Ok(trimmed.to_string())
}

pub fn require_positive_amount(amount: i32) -> Result<i32, ApiError> {
    if amount <= 0 {
        return Err(ApiError::BadRequest("invalid amount".into()));
    }
    Ok(amount)
}

pub fn parse_tag_names(tags: &[String]) -> Result<Vec<String>, ApiError> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let name = tag.trim().to_lowercase();
        if name.is_empty() || !seen.insert(name.clone()) {
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
