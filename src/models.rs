use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use diesel_derive_enum::DbEnum;
use serde::Serialize;

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

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::user_settings)]
pub struct UserSettingsRow {
    pub id: i32,
    pub display_currency: CurrencyCode,
    pub primary_schedule_id: Option<i32>,
    pub projection_initial_free_money: i32,
    pub projection_start_date: Option<NaiveDate>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettingsResponse {
    pub id: i32,
    pub display_currency: CurrencyCode,
    pub primary_schedule_id: Option<i32>,
    pub projection_initial_free_money: i32,
    pub projection_start_date: Option<NaiveDate>,
    pub updated_at: DateTime<Utc>,
}

impl From<UserSettingsRow> for UserSettingsResponse {
    fn from(row: UserSettingsRow) -> Self {
        Self {
            id: row.id,
            display_currency: row.display_currency,
            primary_schedule_id: row.primary_schedule_id,
            projection_initial_free_money: row.projection_initial_free_money,
            projection_start_date: row.projection_start_date,
            updated_at: row.updated_at,
        }
    }
}
