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
    period.start_date <= end && period.end_date >= start
}

pub fn get_budget_projection_amount(
    amount: i32,
    end_date: Option<NaiveDate>,
    spent: i32,
    today: NaiveDate,
    completed_at: Option<&DateTime<Utc>>,
) -> i32 {
    if completed_at.is_some() {
        return spent;
    }
    let Some(end) = end_date else {
        return 0;
    };
    if today <= end {
        amount
    } else {
        spent
    }
}

pub fn is_budget_projection_projected(
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    today: NaiveDate,
) -> bool {
    let (Some(start), Some(end)) = (start_date, end_date) else {
        return false;
    };
    today <= end && today < start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_budget_uses_spent_even_on_end_date() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        let completed = Some(Utc::now());
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(), completed.as_ref()),
            4_500
        );
    }

    #[test]
    fn active_dated_budget_uses_envelope_on_last_day() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(), None),
            10_000
        );
    }

    #[test]
    fn ended_budget_uses_spent_after_end_date() {
        let end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        assert_eq!(
            get_budget_projection_amount(10_000, Some(end), 4_500, NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(), None),
            4_500
        );
    }
}
