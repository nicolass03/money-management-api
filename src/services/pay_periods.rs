use chrono::{Datelike, NaiveDate};

use crate::models::{IncomePayScheduleRow, PayFrequency, RecurringExpenseRow};

pub const PROJECTION_MONTHS_FORWARD: u32 = 12;

#[derive(Debug, Clone)]
pub struct PayPeriod {
    pub pay_date: String,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Clone)]
pub struct PayScheduleInput {
    pub anchor_date: String,
    pub frequency: PayFrequency,
    pub last_payment_date: Option<String>,
}

pub fn schedule_from_income(schedule: &IncomePayScheduleRow) -> PayScheduleInput {
    PayScheduleInput {
        anchor_date: schedule.anchor_date.format("%Y-%m-%d").to_string(),
        frequency: schedule.frequency,
        last_payment_date: None,
    }
}

pub fn schedule_from_recurring(recurring: &RecurringExpenseRow) -> PayScheduleInput {
    PayScheduleInput {
        anchor_date: recurring.anchor_date.format("%Y-%m-%d").to_string(),
        frequency: recurring.frequency,
        last_payment_date: recurring
            .last_payment_date
            .map(|d| d.format("%Y-%m-%d").to_string()),
    }
}

fn parse_date(iso: &str) -> (i32, u32, u32) {
    let date = NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("valid date");
    (date.year(), date.month(), date.day())
}

fn to_iso(y: i32, m: u32, d: u32) -> String {
    format!("{y}-{m:02}-{d:02}")
}

pub fn add_days(iso: &str, days: i32) -> String {
    let date = NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("valid date");
    (date + chrono::Duration::days(i64::from(days)))
        .format("%Y-%m-%d")
        .to_string()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    NaiveDate::from_ymd_opt(year, month + 1, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
        .pred_opt()
        .unwrap()
        .day()
}

fn clamp_day_of_month(year: i32, month: u32, day: u32) -> u32 {
    day.min(days_in_month(year, month))
}

pub fn add_months(iso: &str, months: i32) -> String {
    let (y, m, d) = parse_date(iso);
    let total_months = y * 12 + i32::try_from(m - 1).unwrap() + months;
    let new_y = total_months.div_euclid(12);
    let new_m = u32::try_from(total_months.rem_euclid(12) + 1).unwrap();
    to_iso(new_y, new_m, clamp_day_of_month(new_y, new_m, d))
}

fn monthly_pay_date(year: i32, month: u32, anchor_day: u32) -> String {
    let d = clamp_day_of_month(year, month, anchor_day);
    to_iso(year, month, d)
}

pub fn compare_iso(a: &str, b: &str) -> i32 {
    a.cmp(b) as i32
}

fn days_between(start_iso: &str, end_iso: &str) -> i32 {
    let start = NaiveDate::parse_from_str(start_iso, "%Y-%m-%d").unwrap();
    let end = NaiveDate::parse_from_str(end_iso, "%Y-%m-%d").unwrap();
    i32::try_from((end - start).num_days()).unwrap()
}

fn get_effective_end_date(schedule: &PayScheduleInput, end_date: &str) -> String {
    if let Some(ref last) = schedule.last_payment_date {
        if compare_iso(last, end_date) < 0 {
            return last.clone();
        }
    }
    end_date.to_string()
}

fn get_interval_days(frequency: PayFrequency) -> Option<i32> {
    match frequency {
        PayFrequency::Weekly => Some(7),
        PayFrequency::Biweekly => Some(14),
        _ => None,
    }
}

fn get_next_interval_pay_date(anchor: &str, from_date: &str, interval_days: i32) -> String {
    if compare_iso(from_date, anchor) <= 0 {
        return anchor.to_string();
    }
    let diff = days_between(anchor, from_date);
    let remainder = diff % interval_days;
    if remainder == 0 {
        return from_date.to_string();
    }
    add_days(from_date, interval_days - remainder)
}

fn get_next_monthly_pay_date(anchor_date: &str, from_date: &str) -> String {
    let anchor_day = parse_date(anchor_date).2;
    let (y, m, _) = parse_date(from_date);
    let this_month_pay = monthly_pay_date(y, m, anchor_day);
    if compare_iso(&this_month_pay, from_date) >= 0 {
        return this_month_pay;
    }
    let (new_y, new_m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    monthly_pay_date(new_y, new_m, anchor_day)
}

fn get_next_yearly_pay_date(anchor_date: &str, from_date: &str) -> String {
    let (_, anchor_month, anchor_day) = parse_date(anchor_date);
    let (y, _, _) = parse_date(from_date);
    let this_year_pay = to_iso(
        y,
        anchor_month,
        clamp_day_of_month(y, anchor_month, anchor_day),
    );
    if compare_iso(&this_year_pay, from_date) >= 0 {
        return this_year_pay;
    }
    to_iso(
        y + 1,
        anchor_month,
        clamp_day_of_month(y + 1, anchor_month, anchor_day),
    )
}

fn advance_pay_date(schedule: &PayScheduleInput, current: &str) -> String {
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return add_days(current, interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        return add_months(current, 12);
    }
    let anchor_day = parse_date(&schedule.anchor_date).2;
    let (y, m, _) = parse_date(current);
    let (new_y, new_m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    monthly_pay_date(new_y, new_m, anchor_day)
}

pub fn get_next_pay_date(schedule: &PayScheduleInput, from_date: &str) -> String {
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return get_next_interval_pay_date(&schedule.anchor_date, from_date, interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        return get_next_yearly_pay_date(&schedule.anchor_date, from_date);
    }
    get_next_monthly_pay_date(&schedule.anchor_date, from_date)
}

pub fn get_previous_pay_date(schedule: &PayScheduleInput, pay_date: &str) -> String {
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return add_days(pay_date, -interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        let (y, m, d) = parse_date(pay_date);
        return to_iso(y - 1, m, clamp_day_of_month(y - 1, m, d));
    }
    let anchor_day = parse_date(&schedule.anchor_date).2;
    let (y, m, _) = parse_date(pay_date);
    let (new_y, new_m) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    monthly_pay_date(new_y, new_m, anchor_day)
}

pub fn get_period_containing(schedule: &PayScheduleInput, date: &str) -> PayPeriod {
    let pay_date = get_next_pay_date(schedule, date);
    let previous_pay_date = get_previous_pay_date(schedule, &pay_date);
    PayPeriod {
        pay_date: pay_date.clone(),
        start_date: add_days(&previous_pay_date, 1),
        end_date: pay_date,
    }
}

pub fn get_pay_dates_in_range(
    schedule: &PayScheduleInput,
    start_date: &str,
    end_date: &str,
) -> Vec<String> {
    let effective_end = get_effective_end_date(schedule, end_date);
    if compare_iso(start_date, &effective_end) > 0 {
        return Vec::new();
    }

    let mut dates = Vec::new();
    let mut current = get_next_pay_date(schedule, start_date);

    while compare_iso(&current, &effective_end) <= 0 {
        if compare_iso(&current, start_date) >= 0 {
            dates.push(current.clone());
        }
        current = advance_pay_date(schedule, &current);
    }

    dates
}

pub fn is_date_in_period(date: &str, period: &PayPeriod) -> bool {
    compare_iso(date, &period.start_date) >= 0 && compare_iso(date, &period.end_date) <= 0
}

pub fn get_period_for_pay_date(schedule: &PayScheduleInput, pay_date: &str) -> PayPeriod {
    let previous_pay_date = get_previous_pay_date(schedule, pay_date);
    PayPeriod {
        pay_date: pay_date.to_string(),
        start_date: add_days(&previous_pay_date, 1),
        end_date: pay_date.to_string(),
    }
}

pub fn get_projection_periods(
    schedule: &PayScheduleInput,
    reference_date: Option<&str>,
    projection_start_date: Option<&str>,
    months_forward: u32,
) -> Vec<PayPeriod> {
    let ref_date = reference_date.map(str::to_string).unwrap_or_else(|| {
        let today = chrono::Utc::now().date_naive();
        today.format("%Y-%m-%d").to_string()
    });

    let horizon_end = add_months(&ref_date, i32::try_from(months_forward).unwrap());
    let range_start = projection_start_date.unwrap_or(&ref_date);
    let anchor_date = if let Some(start) = projection_start_date {
        if compare_iso(start, &ref_date) < 0 {
            start
        } else {
            &ref_date
        }
    } else {
        &ref_date
    };

    let anchor_period = get_period_containing(schedule, anchor_date);
    let mut period_map = std::collections::BTreeMap::new();
    let mut pay_date = anchor_period.pay_date;

    while period_map.len() < 50 {
        let period = get_period_for_pay_date(schedule, &pay_date);
        if compare_iso(&period.start_date, &horizon_end) > 0 {
            break;
        }

        let overlaps_horizon = compare_iso(&period.end_date, range_start) >= 0
            && compare_iso(&period.start_date, &horizon_end) <= 0;

        if overlaps_horizon {
            period_map.insert(period.pay_date.clone(), period);
        }

        pay_date = get_next_pay_date(schedule, &add_days(&pay_date, 1));
    }

    period_map.into_values().collect()
}
