use chrono::{Datelike, Duration, Months, NaiveDate};

use crate::models::{IncomePayScheduleRow, PayFrequency, RecurringExpenseRow};

pub const DEFAULT_PROJECTION_MONTHS_FORWARD: i32 = 12;
const MAX_PROJECTION_PERIODS: usize = 5_000;

/// An inclusive date range `[start_date, end_date]` closing on `pay_date`. For pay periods
/// `pay_date == end_date`; plain ranges (reports, calendar views) use [`PayPeriod::range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayPeriod {
    pub pay_date: NaiveDate,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

impl PayPeriod {
    pub fn range(start_date: NaiveDate, end_date: NaiveDate) -> Self {
        Self {
            pay_date: end_date,
            start_date,
            end_date,
        }
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start_date <= date && date <= self.end_date
    }

    /// This period narrowed to `[from, to]`, keeping its pay date.
    pub fn clip(&self, from: NaiveDate, to: NaiveDate) -> Self {
        Self {
            pay_date: self.pay_date,
            start_date: self.start_date.max(from),
            end_date: self.end_date.min(to),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PayScheduleInput {
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
    pub last_payment_date: Option<NaiveDate>,
}

pub fn schedule_from_income(schedule: &IncomePayScheduleRow) -> PayScheduleInput {
    PayScheduleInput {
        anchor_date: schedule.anchor_date,
        frequency: schedule.frequency,
        last_payment_date: None,
    }
}

pub fn schedule_from_recurring(recurring: &RecurringExpenseRow) -> PayScheduleInput {
    PayScheduleInput {
        anchor_date: recurring.anchor_date,
        frequency: recurring.frequency,
        last_payment_date: recurring.last_payment_date,
    }
}

pub fn add_days(date: NaiveDate, days: i64) -> NaiveDate {
    date + Duration::days(days)
}

/// Calendar month arithmetic, clamping the day to the target month's length (Jan 31 + 1 month =
/// Feb 28/29).
pub fn add_months(date: NaiveDate, months: i32) -> NaiveDate {
    let delta = Months::new(months.unsigned_abs());
    if months >= 0 {
        date.checked_add_months(delta)
    } else {
        date.checked_sub_months(delta)
    }
    .expect("date stays within chrono's supported range")
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let next = first + Months::new(1);
    (next - first).num_days() as u32
}

/// `day` of the given month, clamped to the month's last day.
fn clamped_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day.min(days_in_month(year, month))).expect("valid date")
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn previous_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn get_effective_end_date(schedule: &PayScheduleInput, end_date: NaiveDate) -> NaiveDate {
    schedule
        .last_payment_date
        .map_or(end_date, |last| last.min(end_date))
}

fn get_interval_days(frequency: PayFrequency) -> Option<i64> {
    match frequency {
        PayFrequency::Weekly => Some(7),
        PayFrequency::Biweekly => Some(14),
        _ => None,
    }
}

/// Next grid date on or after `from_date`. Unbounded by the anchor (works before it too), so the
/// pay-period grid is well-defined for any date; occurrence lookups clamp to the anchor separately.
fn get_next_interval_pay_date(anchor: NaiveDate, from_date: NaiveDate, interval_days: i64) -> NaiveDate {
    let remainder = (from_date - anchor).num_days().rem_euclid(interval_days);
    if remainder == 0 {
        return from_date;
    }
    add_days(from_date, interval_days - remainder)
}

fn get_next_monthly_pay_date(anchor_date: NaiveDate, from_date: NaiveDate) -> NaiveDate {
    let this_month_pay = clamped_date(from_date.year(), from_date.month(), anchor_date.day());
    if this_month_pay >= from_date {
        return this_month_pay;
    }
    let (year, month) = next_month(from_date.year(), from_date.month());
    clamped_date(year, month, anchor_date.day())
}

fn get_next_yearly_pay_date(anchor_date: NaiveDate, from_date: NaiveDate) -> NaiveDate {
    let this_year_pay = clamped_date(from_date.year(), anchor_date.month(), anchor_date.day());
    if this_year_pay >= from_date {
        return this_year_pay;
    }
    clamped_date(from_date.year() + 1, anchor_date.month(), anchor_date.day())
}

fn advance_pay_date(schedule: &PayScheduleInput, current: NaiveDate) -> NaiveDate {
    let anchor = schedule.anchor_date;
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return add_days(current, interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        // Re-derive from the anchor's month/day so a Feb 29 anchor returns to the 29th in leap
        // years instead of drifting to the 28th forever.
        return clamped_date(current.year() + 1, anchor.month(), anchor.day());
    }
    let (year, month) = next_month(current.year(), current.month());
    clamped_date(year, month, anchor.day())
}

/// Next actual occurrence (a real charge or payday) on or after `from_date`. Unlike
/// [`get_next_pay_date`], never returns a date before the schedule's anchor (its first occurrence),
/// so a schedule starting in the future doesn't fire early.
pub fn get_next_occurrence(schedule: &PayScheduleInput, from_date: NaiveDate) -> NaiveDate {
    get_next_pay_date(schedule, from_date.max(schedule.anchor_date))
}

/// Next date on the schedule's pay-period grid on or after `from_date`. Unbounded by the anchor:
/// used for period boundaries, which must exist for any date. For real occurrences use
/// [`get_next_occurrence`] / [`get_pay_dates_in_range`].
pub fn get_next_pay_date(schedule: &PayScheduleInput, from_date: NaiveDate) -> NaiveDate {
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return get_next_interval_pay_date(schedule.anchor_date, from_date, interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        return get_next_yearly_pay_date(schedule.anchor_date, from_date);
    }
    get_next_monthly_pay_date(schedule.anchor_date, from_date)
}

pub fn get_previous_pay_date(schedule: &PayScheduleInput, pay_date: NaiveDate) -> NaiveDate {
    let anchor = schedule.anchor_date;
    if let Some(interval_days) = get_interval_days(schedule.frequency) {
        return add_days(pay_date, -interval_days);
    }
    if schedule.frequency == PayFrequency::Yearly {
        return clamped_date(pay_date.year() - 1, anchor.month(), anchor.day());
    }
    let (year, month) = previous_month(pay_date.year(), pay_date.month());
    clamped_date(year, month, anchor.day())
}

pub fn get_period_containing(schedule: &PayScheduleInput, date: NaiveDate) -> PayPeriod {
    get_period_for_pay_date(schedule, get_next_pay_date(schedule, date))
}

pub fn get_pay_dates_in_range(
    schedule: &PayScheduleInput,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Vec<NaiveDate> {
    let effective_end = get_effective_end_date(schedule, end_date);
    if start_date > effective_end {
        return Vec::new();
    }

    let mut dates = Vec::new();
    // Occurrences never precede the anchor (the schedule's first charge/payday).
    let mut current = get_next_occurrence(schedule, start_date);

    while current <= effective_end {
        if current >= start_date {
            dates.push(current);
        }
        current = advance_pay_date(schedule, current);
    }

    dates
}

/// Whether the schedule has an occurrence on `date`.
pub fn occurs_on(schedule: &PayScheduleInput, date: NaiveDate) -> bool {
    !get_pay_dates_in_range(schedule, date, date).is_empty()
}

pub fn get_period_for_pay_date(schedule: &PayScheduleInput, pay_date: NaiveDate) -> PayPeriod {
    PayPeriod {
        pay_date,
        start_date: add_days(get_previous_pay_date(schedule, pay_date), 1),
        end_date: pay_date,
    }
}

pub fn get_projection_periods(
    schedule: &PayScheduleInput,
    reference_date: NaiveDate,
    projection_start_date: Option<NaiveDate>,
    projection_end_date: Option<NaiveDate>,
) -> Vec<PayPeriod> {
    let horizon_end = projection_end_date
        .unwrap_or_else(|| add_months(reference_date, DEFAULT_PROJECTION_MONTHS_FORWARD));
    let range_start = projection_start_date.unwrap_or(reference_date);
    let anchor_date = projection_start_date.map_or(reference_date, |start| start.min(reference_date));

    let mut periods = Vec::new();
    let mut pay_date = get_period_containing(schedule, anchor_date).pay_date;

    for _ in 0..MAX_PROJECTION_PERIODS {
        let period = get_period_for_pay_date(schedule, pay_date);
        if period.start_date > horizon_end {
            break;
        }

        let overlaps_horizon = period.end_date >= range_start && period.start_date <= horizon_end;
        if overlaps_horizon {
            periods.push(period);
        }

        let next_pay_date = get_next_pay_date(schedule, add_days(pay_date, 1));
        if next_pay_date <= pay_date {
            break;
        }
        pay_date = next_pay_date;
    }

    periods
}

/// Pay periods whose `[start_date, end_date]` overlaps `[from, to]` (inclusive), in pay-date order.
pub fn get_pay_periods_overlapping_range(
    schedule: &PayScheduleInput,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<PayPeriod> {
    if from > to {
        return Vec::new();
    }

    let mut periods = Vec::new();
    let mut pay_date = get_period_containing(schedule, from).pay_date;

    for _ in 0..400 {
        let period = get_period_for_pay_date(schedule, pay_date);

        if period.start_date > to {
            break;
        }

        if period.end_date >= from {
            periods.push(period);
        }

        pay_date = get_next_pay_date(schedule, add_days(pay_date, 1));
    }

    periods
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PayFrequency;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn monthly_schedule() -> PayScheduleInput {
        PayScheduleInput {
            anchor_date: date("2026-01-25"),
            frequency: PayFrequency::Monthly,
            last_payment_date: None,
        }
    }

    #[test]
    fn period_on_payday_is_closing_period() {
        let period = get_period_containing(&monthly_schedule(), date("2026-06-25"));
        assert_eq!(period.start_date, date("2026-05-26"));
        assert_eq!(period.end_date, date("2026-06-25"));
        assert_eq!(period.pay_date, date("2026-06-25"));
    }

    #[test]
    fn period_day_after_payday_rolls_forward() {
        let period = get_period_containing(&monthly_schedule(), date("2026-06-26"));
        assert_eq!(period.start_date, date("2026-06-26"));
        assert_eq!(period.end_date, date("2026-07-25"));
        assert_eq!(period.pay_date, date("2026-07-25"));
    }

    #[test]
    fn overlapping_pay_periods_cover_range() {
        let schedule = monthly_schedule();
        let periods =
            get_pay_periods_overlapping_range(&schedule, date("2026-03-01"), date("2026-05-31"));
        assert_eq!(periods.len(), 4);
        assert_eq!(periods[0].pay_date, date("2026-03-25"));
        assert_eq!(periods[2].pay_date, date("2026-05-25"));
        assert_eq!(periods[3].pay_date, date("2026-06-25"));
    }

    #[test]
    fn projection_default_remains_twelve_months() {
        let periods = get_projection_periods(&monthly_schedule(), date("2026-06-01"), None, None);

        assert_eq!(periods.first().unwrap().pay_date, date("2026-06-25"));
        assert_eq!(periods.last().unwrap().pay_date, date("2027-06-25"));
    }

    #[test]
    fn projection_end_includes_its_full_pay_period() {
        let periods = get_projection_periods(
            &monthly_schedule(),
            date("2026-06-01"),
            None,
            Some(date("2026-10-10")),
        );

        assert_eq!(periods.last().unwrap().start_date, date("2026-09-26"));
        assert_eq!(periods.last().unwrap().end_date, date("2026-10-25"));
    }

    #[test]
    fn two_year_weekly_projection_is_not_capped_at_fifty_periods() {
        let schedule = PayScheduleInput {
            anchor_date: date("2026-09-12"),
            frequency: PayFrequency::Weekly,
            last_payment_date: None,
        };
        let periods = get_projection_periods(
            &schedule,
            date("2026-09-12"),
            None,
            Some(date("2028-09-12")),
        );

        assert!(periods.len() > 100);
        assert!(periods.last().unwrap().end_date >= date("2028-09-12"));
    }
}
