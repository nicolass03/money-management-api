use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{CurrencyCode, IncomePayScheduleResponse};
use crate::services::currency::ExchangeRates;
use crate::services::projections::ProjectionRow;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchSettingsRequest {
    pub display_currency: Option<String>,
    pub primary_schedule_id: Option<Option<Uuid>>,
    pub projection_initial_free_money: Option<i32>,
    pub projection_start_date: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyContextQuery {
    #[serde(default)]
    pub force_refresh: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyContextResponse {
    pub display_currency: CurrencyCode,
    pub rates: ExchangeRates,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIncomeScheduleRequest {
    pub name: String,
    pub anchor_date: String,
    pub frequency: String,
    pub amount: i32,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIncomeScheduleRequest {
    pub name: String,
    pub anchor_date: String,
    pub frequency: String,
    pub amount: i32,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIncomeRequest {
    pub name: String,
    pub amount: i32,
    pub currency: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIncomeRequest {
    pub name: String,
    pub amount: i32,
    pub currency: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExpenseRequest {
    pub name: String,
    pub amount: i32,
    pub currency: String,
    pub date: String,
    pub tags: Vec<String>,
    pub is_subscription: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchExpenseRequest {
    pub amount: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EarlyPayExpenseRequest {
    pub source_type: String,
    pub scheduled_date: String,
    pub paid_date: String,
    pub amount: i32,
    pub currency: String,
    pub recurring_id: Option<Uuid>,
    pub planned_expense_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecurringExpenseRequest {
    pub name: String,
    pub anchor_date: String,
    pub frequency: String,
    pub amount: i32,
    pub currency: String,
    pub tags: Vec<String>,
    pub is_subscription: bool,
    pub last_payment_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecurringExpenseRequest {
    pub name: String,
    pub anchor_date: String,
    pub frequency: String,
    pub amount: i32,
    pub currency: String,
    pub tags: Vec<String>,
    pub is_subscription: bool,
    pub last_payment_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlannedExpenseRequest {
    pub name: String,
    pub date: String,
    pub amount: i32,
    pub currency: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlannedExpenseRequest {
    pub name: String,
    pub date: String,
    pub amount: i32,
    pub currency: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBudgetRequest {
    pub name: String,
    pub amount: i32,
    pub currency: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBudgetRequest {
    pub name: String,
    pub amount: i32,
    pub currency: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBudgetExpenseRequest {
    pub name: Option<String>,
    pub amount: i32,
    pub date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionsResponse {
    pub rows: Vec<ProjectionRow>,
    pub primary_schedule: IncomePayScheduleResponse,
    pub display_currency: CurrencyCode,
    pub rates: ExchangeRates,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyExpensesResponse {
    pub date: String,
    pub created: i32,
}
