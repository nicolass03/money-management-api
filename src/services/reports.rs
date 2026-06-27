use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use uuid::Uuid;

use crate::models::{BudgetRow, CurrencyCode, ExpenseRow, IncomeRow};
use crate::services::currency::{convert_amount, ExchangeRates};
use crate::services::expense_period::{
    build_chart_summary, compute_extra_spent, SubscriptionSplit, TagAmountEntry,
};
use crate::services::pay_periods::{add_days, is_date_in_period, PayPeriod};

pub const MAX_REPORT_RANGE_DAYS: i64 = 730;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportTimeGranularity {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRange {
    pub from: String,
    pub to: String,
    pub day_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportKpis {
    pub total_income: i32,
    pub total_expenses: i32,
    pub net_cash_flow: i32,
    pub extra_spent: i32,
    pub expense_count: i32,
    pub income_count: i32,
    pub avg_daily_spend: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPriorPeriod {
    pub from: String,
    pub to: String,
    pub kpis: ReportKpis,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTimeBucket {
    pub start_date: String,
    pub end_date: String,
    pub label: String,
    pub income: i32,
    pub expenses: i32,
    pub net: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTimeSeries {
    pub granularity: ReportTimeGranularity,
    pub buckets: Vec<ReportTimeBucket>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportBudgetSpend {
    pub budget_id: Uuid,
    pub name: String,
    pub amount: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummaryResponse {
    pub range: ReportRange,
    pub display_currency: CurrencyCode,
    pub rates: ExchangeRates,
    pub kpis: ReportKpis,
    pub prior_period: Option<ReportPriorPeriod>,
    pub by_tag: Vec<TagAmountEntry>,
    pub subscription_split: SubscriptionSplit,
    pub time_series: ReportTimeSeries,
    pub top_budgets: Vec<ReportBudgetSpend>,
}

pub fn validate_report_range(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate, i64), String> {
    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d")
        .map_err(|_| "invalid from date".to_string())?;
    let to_date =
        NaiveDate::parse_from_str(to, "%Y-%m-%d").map_err(|_| "invalid to date".to_string())?;
    if from_date > to_date {
        return Err("from must be on or before to".into());
    }
    let day_count = (to_date - from_date).num_days() + 1;
    if day_count > MAX_REPORT_RANGE_DAYS {
        return Err(format!(
            "date range cannot exceed {MAX_REPORT_RANGE_DAYS} days"
        ));
    }
    Ok((from_date, to_date, day_count))
}

pub fn prior_period_range(from: &str, _to: &str, day_count: i64) -> (String, String) {
    let prior_to = add_days(from, -1);
    let prior_from = add_days(&prior_to, -i32::try_from(day_count - 1).unwrap_or(0));
    (prior_from, prior_to)
}

pub fn resolve_granularity(day_count: i64) -> ReportTimeGranularity {
    if day_count <= 31 {
        ReportTimeGranularity::Day
    } else if day_count <= 120 {
        ReportTimeGranularity::Week
    } else {
        ReportTimeGranularity::Month
    }
}

pub fn build_time_buckets(
    from: &str,
    to: &str,
    granularity: ReportTimeGranularity,
) -> Vec<(String, String, String)> {
    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d").expect("valid from");
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d").expect("valid to");

    match granularity {
        ReportTimeGranularity::Day => {
            let mut buckets = Vec::new();
            let mut cursor = from_date;
            while cursor <= to_date {
                let iso = cursor.format("%Y-%m-%d").to_string();
                buckets.push((iso.clone(), iso.clone(), iso));
                cursor += chrono::Duration::days(1);
            }
            buckets
        }
        ReportTimeGranularity::Week => {
            let mut buckets = Vec::new();
            let mut cursor = week_start(from_date);
            while cursor <= to_date {
                let week_end = (cursor + chrono::Duration::days(6)).min(to_date);
                let start = cursor.max(from_date);
                let start_iso = start.format("%Y-%m-%d").to_string();
                let end_iso = week_end.format("%Y-%m-%d").to_string();
                let label = format!("{start_iso} – {end_iso}");
                buckets.push((start_iso, end_iso, label));
                cursor += chrono::Duration::days(7);
            }
            buckets
        }
        ReportTimeGranularity::Month => {
            let mut buckets = Vec::new();
            let mut year = from_date.year();
            let mut month = from_date.month();
            loop {
                let month_start =
                    NaiveDate::from_ymd_opt(year, month, 1).expect("valid month start");
                if month_start > to_date {
                    break;
                }
                let next_month = if month == 12 {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(year, month + 1, 1)
                }
                .expect("valid next month");
                let month_end = (next_month - chrono::Duration::days(1)).min(to_date);
                let start = month_start.max(from_date);
                let start_iso = start.format("%Y-%m-%d").to_string();
                let end_iso = month_end.format("%Y-%m-%d").to_string();
                let label = format!("{year}-{month:02}");
                buckets.push((start_iso, end_iso, label));
                if month == 12 {
                    year += 1;
                    month = 1;
                } else {
                    month += 1;
                }
            }
            buckets
        }
    }
}

fn week_start(date: NaiveDate) -> NaiveDate {
    let weekday = date.weekday().num_days_from_monday();
    date - chrono::Duration::days(i64::from(weekday))
}

fn pay_period(from: &str, to: &str) -> PayPeriod {
    PayPeriod {
        pay_date: to.to_string(),
        start_date: from.to_string(),
        end_date: to.to_string(),
    }
}

fn sum_income_in_period(
    income_rows: &[IncomeRow],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> (i32, i32) {
    let mut total = 0i32;
    let mut count = 0i32;
    for row in income_rows {
        let date = row.date.format("%Y-%m-%d").to_string();
        if is_date_in_period(&date, period) {
            total += convert_amount(row.amount, row.currency, display_currency, rates);
            count += 1;
        }
    }
    (total, count)
}

fn sum_expenses_in_period(
    expenses: &[(ExpenseRow, Vec<String>)],
    period: &PayPeriod,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> (i32, i32) {
    let mut total = 0i32;
    let mut count = 0i32;
    for (row, _) in expenses {
        let date = row.date.format("%Y-%m-%d").to_string();
        if is_date_in_period(&date, period) {
            total += convert_amount(row.amount, row.currency, display_currency, rates);
            count += 1;
        }
    }
    (total, count)
}

pub fn compute_kpis(
    expenses: &[(ExpenseRow, Vec<String>)],
    income_rows: &[IncomeRow],
    from: &str,
    to: &str,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
    day_count: i64,
) -> ReportKpis {
    let period = pay_period(from, to);
    let chart = build_chart_summary(expenses, from, to, display_currency, rates);
    let total_expenses: i32 = chart.by_tag.iter().map(|entry| entry.amount).sum();
    let expense_count = expenses
        .iter()
        .filter(|(row, _)| {
            let date = row.date.format("%Y-%m-%d").to_string();
            is_date_in_period(&date, &period)
        })
        .count() as i32;
    let (total_income, income_count) =
        sum_income_in_period(income_rows, &period, display_currency, rates);
    let extra_spent = compute_extra_spent(expenses, &period, display_currency, rates);
    let avg_daily_spend = if day_count > 0 {
        total_expenses / i32::try_from(day_count).unwrap_or(1)
    } else {
        0
    };

    ReportKpis {
        total_income,
        total_expenses,
        net_cash_flow: total_income - total_expenses,
        extra_spent,
        expense_count,
        income_count,
        avg_daily_spend,
    }
}

fn build_time_series(
    expenses: &[(ExpenseRow, Vec<String>)],
    income_rows: &[IncomeRow],
    from: &str,
    to: &str,
    granularity: ReportTimeGranularity,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> ReportTimeSeries {
    let bucket_defs = build_time_buckets(from, to, granularity);
    let buckets = bucket_defs
        .into_iter()
        .map(|(start_date, end_date, label)| {
            let period = pay_period(&start_date, &end_date);
            let (income, _) = sum_income_in_period(income_rows, &period, display_currency, rates);
            let (expenses, _) =
                sum_expenses_in_period(expenses, &period, display_currency, rates);
            ReportTimeBucket {
                start_date,
                end_date,
                label,
                income,
                expenses,
                net: income - expenses,
            }
        })
        .collect();

    ReportTimeSeries {
        granularity,
        buckets,
    }
}

fn build_top_budgets(
    expenses: &[(ExpenseRow, Vec<String>)],
    budgets: &[BudgetRow],
    from: &str,
    to: &str,
    display_currency: CurrencyCode,
    rates: &ExchangeRates,
) -> Vec<ReportBudgetSpend> {
    let period = pay_period(from, to);
    let budget_names: HashMap<Uuid, &str> = budgets
        .iter()
        .map(|b| (b.id, b.name.as_str()))
        .collect();
    let mut totals: HashMap<Uuid, i32> = HashMap::new();

    for (row, _) in expenses {
        let Some(budget_id) = row.budget_id else {
            continue;
        };
        let date = row.date.format("%Y-%m-%d").to_string();
        if !is_date_in_period(&date, &period) {
            continue;
        }
        let converted = convert_amount(row.amount, row.currency, display_currency, rates);
        *totals.entry(budget_id).or_insert(0) += converted;
    }

    let mut entries: Vec<ReportBudgetSpend> = totals
        .into_iter()
        .filter_map(|(budget_id, amount)| {
            budget_names.get(&budget_id).map(|name| ReportBudgetSpend {
                budget_id,
                name: (*name).to_string(),
                amount,
            })
        })
        .collect();
    entries.sort_by(|a, b| b.amount.cmp(&a.amount));
    entries.truncate(5);
    entries
}

pub fn build_report_summary(
    from: &str,
    to: &str,
    expenses: &[(ExpenseRow, Vec<String>)],
    income_rows: &[IncomeRow],
    budgets: &[BudgetRow],
    display_currency: CurrencyCode,
    rates: ExchangeRates,
    compare_prior: bool,
) -> Result<ReportSummaryResponse, String> {
    let (_from_date, _to_date, day_count) = validate_report_range(from, to)?;
    let granularity = resolve_granularity(day_count);
    let chart = build_chart_summary(expenses, from, to, display_currency, &rates);
    let kpis = compute_kpis(
        expenses,
        income_rows,
        from,
        to,
        display_currency,
        &rates,
        day_count,
    );

    let prior_period = if compare_prior {
        let (prior_from, prior_to) = prior_period_range(from, to, day_count);
        let prior_kpis = compute_kpis(
            expenses,
            income_rows,
            &prior_from,
            &prior_to,
            display_currency,
            &rates,
            day_count,
        );
        Some(ReportPriorPeriod {
            from: prior_from,
            to: prior_to,
            kpis: prior_kpis,
        })
    } else {
        None
    };

    let time_series = build_time_series(
        expenses,
        income_rows,
        from,
        to,
        granularity,
        display_currency,
        &rates,
    );
    let top_budgets = build_top_budgets(
        expenses,
        budgets,
        from,
        to,
        display_currency,
        &rates,
    );

    Ok(ReportSummaryResponse {
        range: ReportRange {
            from: from.to_string(),
            to: to.to_string(),
            day_count,
        },
        display_currency,
        rates,
        kpis,
        prior_period,
        by_tag: chart.by_tag,
        subscription_split: chart.subscription_split,
        time_series,
        top_budgets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_range_rejects_inverted_dates() {
        let err = validate_report_range("2025-02-01", "2025-01-01").unwrap_err();
        assert!(err.contains("from must be on or before to"));
    }

    #[test]
    fn validate_range_rejects_too_long() {
        let err = validate_report_range("2020-01-01", "2025-01-01").unwrap_err();
        assert!(err.contains("730"));
    }

    #[test]
    fn prior_period_is_equal_length() {
        let (from, to) = prior_period_range("2025-02-01", "2025-03-02", 30);
        assert_eq!(from, "2025-01-02");
        assert_eq!(to, "2025-01-31");
    }

    #[test]
    fn granularity_thresholds() {
        assert_eq!(resolve_granularity(31), ReportTimeGranularity::Day);
        assert_eq!(resolve_granularity(32), ReportTimeGranularity::Week);
        assert_eq!(resolve_granularity(120), ReportTimeGranularity::Week);
        assert_eq!(resolve_granularity(121), ReportTimeGranularity::Month);
    }

    #[test]
    fn daily_bucket_count_matches_range() {
        let buckets = build_time_buckets(
            "2025-01-01",
            "2025-01-07",
            ReportTimeGranularity::Day,
        );
        assert_eq!(buckets.len(), 7);
    }
}
