use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use diesel_derive_enum::DbEnum;
use serde::Serialize;
use uuid::Uuid;

use crate::schema::sql_types;

#[derive(Debug, Clone, Copy, PartialEq, Eq, DbEnum, Serialize)]
#[ExistingTypePath = "sql_types::CurrencyCode"]
#[serde(rename_all = "lowercase")]
pub enum CurrencyCode {
    #[db_rename = "eur"]
    Eur,
    #[db_rename = "usd"]
    Usd,
    #[db_rename = "cop"]
    Cop,
}

impl CurrencyCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eur => "eur",
            Self::Usd => "usd",
            Self::Cop => "cop",
        }
    }

    pub fn to_iso(self) -> &'static str {
        match self {
            Self::Eur => "EUR",
            Self::Usd => "USD",
            Self::Cop => "COP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, DbEnum, Serialize)]
#[ExistingTypePath = "sql_types::PayFrequency"]
#[serde(rename_all = "lowercase")]
pub enum PayFrequency {
    #[db_rename = "weekly"]
    Weekly,
    #[db_rename = "biweekly"]
    Biweekly,
    #[db_rename = "monthly"]
    Monthly,
    #[db_rename = "yearly"]
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, DbEnum, Serialize)]
#[ExistingTypePath = "sql_types::IncomeSource"]
#[serde(rename_all = "lowercase")]
pub enum IncomeSource {
    #[db_rename = "scheduled"]
    Scheduled,
    #[db_rename = "manual"]
    Manual,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::user_settings)]
pub struct UserSettingsRow {
    pub user_id: Uuid,
    pub display_currency: CurrencyCode,
    pub language: String,
    pub primary_schedule_id: Option<Uuid>,
    pub projection_initial_free_money: i32,
    pub projection_start_date: Option<NaiveDate>,
    pub updated_at: DateTime<Utc>,
    pub cache_revision: i64,
    pub extra_spent_limit: Option<i32>,
    pub theme: String,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::income_pay_schedules)]
pub struct IncomePayScheduleRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::income)]
pub struct IncomeRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub source: IncomeSource,
    pub date: NaiveDate,
    pub schedule_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    #[diesel(column_name = amount_overridden)]
    pub _amount_overridden: bool,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::expenses)]
pub struct ExpenseRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub date: NaiveDate,
    pub scheduled_date: Option<NaiveDate>,
    pub recurring_id: Option<Uuid>,
    pub planned_expense_id: Option<Uuid>,
    pub budget_id: Option<Uuid>,
    pub amount_overridden: bool,
    pub is_subscription: bool,
    pub created_at: DateTime<Utc>,
}

impl ExpenseRow {
    pub fn is_system_generated(&self) -> bool {
        self.recurring_id.is_some() || self.planned_expense_id.is_some()
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::recurring_expenses)]
pub struct RecurringExpenseRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub is_subscription: bool,
    pub last_payment_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::planned_expenses)]
pub struct PlannedExpenseRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub date: NaiveDate,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::budgets)]
pub struct BudgetRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::savings)]
pub struct SavingRow {
    pub id: Uuid,
    #[diesel(column_name = user_id)]
    pub _user_id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub note: Option<String>,
    pub date: NaiveDate,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::exchange_rate_snapshots)]
pub struct ExchangeRateSnapshotRow {
    #[diesel(column_name = id)]
    pub _id: Uuid,
    pub base_currency: CurrencyCode,
    pub rates_json: serde_json::Value,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettingsResponse {
    pub id: Uuid,
    pub display_currency: CurrencyCode,
    pub language: String,
    pub primary_schedule_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_schedule: Option<IncomePayScheduleResponse>,
    pub projection_initial_free_money: i32,
    pub projection_start_date: Option<NaiveDate>,
    pub extra_spent_limit: Option<i32>,
    pub theme: String,
    pub cache_revision: i64,
    pub updated_at: DateTime<Utc>,
}

impl UserSettingsResponse {
    pub fn from_row(
        row: UserSettingsRow,
        primary_schedule: Option<IncomePayScheduleRow>,
    ) -> Self {
        Self {
            id: row.user_id,
            display_currency: row.display_currency,
            language: row.language,
            primary_schedule_id: row.primary_schedule_id,
            primary_schedule: primary_schedule.map(IncomePayScheduleResponse::from),
            projection_initial_free_money: row.projection_initial_free_money,
            projection_start_date: row.projection_start_date,
            extra_spent_limit: row.extra_spent_limit,
            theme: row.theme,
            cache_revision: row.cache_revision,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomePayScheduleResponse {
    pub id: Uuid,
    pub name: String,
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<IncomePayScheduleRow> for IncomePayScheduleResponse {
    fn from(row: IncomePayScheduleRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            anchor_date: row.anchor_date,
            frequency: row.frequency,
            amount: row.amount,
            currency: row.currency,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeResponse {
    pub id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub source: IncomeSource,
    pub date: NaiveDate,
    pub schedule_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl From<IncomeRow> for IncomeResponse {
    fn from(row: IncomeRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            amount: row.amount,
            currency: row.currency,
            source: row.source,
            date: row.date,
            schedule_id: row.schedule_id,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseResponse {
    pub id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub date: NaiveDate,
    pub scheduled_date: Option<NaiveDate>,
    pub recurring_id: Option<Uuid>,
    pub planned_expense_id: Option<Uuid>,
    pub budget_id: Option<Uuid>,
    pub amount_overridden: bool,
    pub is_subscription: bool,
    pub created_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringExpenseResponse {
    pub id: Uuid,
    pub name: String,
    pub anchor_date: NaiveDate,
    pub frequency: PayFrequency,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub is_subscription: bool,
    pub last_payment_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedExpenseResponse {
    pub id: Uuid,
    pub name: String,
    pub date: NaiveDate,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetResponse {
    pub id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub spent: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingResponse {
    pub id: Uuid,
    pub name: String,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub note: Option<String>,
    pub date: NaiveDate,
    pub created_at: DateTime<Utc>,
}

impl From<SavingRow> for SavingResponse {
    fn from(row: SavingRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            amount: row.amount,
            currency: row.currency,
            note: row.note,
            date: row.date,
            created_at: row.created_at,
        }
    }
}

pub fn expense_to_response(row: ExpenseRow, tags: Vec<String>) -> ExpenseResponse {
    ExpenseResponse {
        id: row.id,
        name: row.name,
        amount: row.amount,
        currency: row.currency,
        date: row.date,
        scheduled_date: row.scheduled_date,
        recurring_id: row.recurring_id,
        planned_expense_id: row.planned_expense_id,
        budget_id: row.budget_id,
        amount_overridden: row.amount_overridden,
        is_subscription: row.is_subscription,
        created_at: row.created_at,
        tags,
    }
}

pub fn recurring_to_response(row: RecurringExpenseRow, tags: Vec<String>) -> RecurringExpenseResponse {
    RecurringExpenseResponse {
        id: row.id,
        name: row.name,
        anchor_date: row.anchor_date,
        frequency: row.frequency,
        amount: row.amount,
        currency: row.currency,
        is_subscription: row.is_subscription,
        last_payment_date: row.last_payment_date,
        created_at: row.created_at,
        updated_at: row.updated_at,
        tags,
    }
}

pub fn planned_to_response(row: PlannedExpenseRow, tags: Vec<String>) -> PlannedExpenseResponse {
    PlannedExpenseResponse {
        id: row.id,
        name: row.name,
        date: row.date,
        amount: row.amount,
        currency: row.currency,
        created_at: row.created_at,
        updated_at: row.updated_at,
        tags,
    }
}

pub fn budget_to_response(row: BudgetRow, tags: Vec<String>, spent: i32) -> BudgetResponse {
    BudgetResponse {
        id: row.id,
        name: row.name,
        amount: row.amount,
        currency: row.currency,
        start_date: row.start_date,
        end_date: row.end_date,
        created_at: row.created_at,
        updated_at: row.updated_at,
        tags,
        spent,
    }
}

pub fn is_manual_income(row: &IncomeRow) -> bool {
    row.source != IncomeSource::Scheduled && row.schedule_id.is_none()
}
