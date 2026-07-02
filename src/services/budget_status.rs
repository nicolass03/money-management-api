use chrono::{DateTime, NaiveDate, Utc};

use crate::services::pay_periods::PayPeriod;

pub fn is_dated_budget(start_date: Option<NaiveDate>, end_date: Option<NaiveDate>) -> bool {
    start_date.is_some() && end_date.is_some()
}

pub fn budget_overlaps_period(
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    period: &PayPeriod,
) -> bool {
    let (Some(start), Some(end)) = (start_date, end_date) else {
        return false;
    };
    let start_s = start.format("%Y-%m-%d").to_string();
    let end_s = end.format("%Y-%m-%d").to_string();
    period.start_date <= end_s && period.end_date >= start_s
}

pub fn get_budget_projection_amount(
    amount: i32,
    end_date: Option<NaiveDate>,
    spent: i32,
    today: &str,
    completed_at: Option<&DateTime<Utc>>,
) -> i32 {
    if completed_at.is_some() {
        return spent;
    }
    let Some(end) = end_date else {
        return 0;
    };
    let end_s = end.format("%Y-%m-%d").to_string();
    if today <= end_s.as_str() {
        amount
    } else {
        spent
    }
}

pub fn get_budget_projection_period_date(
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    today: &str,
) -> Option<String> {
    let (Some(start), Some(end)) = (start_date, end_date) else {
        return None;
    };
    let end_s = end.format("%Y-%m-%d").to_string();
    if today <= end_s.as_str() {
        Some(start.format("%Y-%m-%d").to_string())
    } else {
        Some(end_s)
    }
}

pub fn is_budget_projection_projected(
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    today: &str,
) -> bool {
    let (Some(start), Some(end)) = (start_date, end_date) else {
        return false;
    };
    let start_s = start.format("%Y-%m-%d").to_string();
    let end_s = end.format("%Y-%m-%d").to_string();
    today <= end_s.as_str() && today < start_s.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_budget_uses_spent_even_on_end_date() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        let completed = Some(Utc::now());
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, "2026-07-02", completed.as_ref()),
            4_500
        );
    }

    #[test]
    fn active_dated_budget_uses_envelope_on_last_day() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, "2026-07-02", None),
            10_000
        );
    }

    #[test]
    fn ended_budget_uses_spent_after_end_date() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, "2026-07-02", None),
            4_500
        );
    }
}
