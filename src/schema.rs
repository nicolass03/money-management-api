// @generated automatically by Diesel CLI.
// Re-run `diesel print-schema` after DB shape changes.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "currency_code"))]
    pub struct CurrencyCode;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "income_source"))]
    pub struct IncomeSource;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "pay_frequency"))]
    pub struct PayFrequency;
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    budgets (id) {
        id -> Int4,
        name -> Text,
        amount -> Int4,
        currency -> CurrencyCode,
        start_date -> Nullable<Date>,
        end_date -> Nullable<Date>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    budget_tags (budget_id, tag_id) {
        budget_id -> Int4,
        tag_id -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    exchange_rate_snapshots (id) {
        id -> Int4,
        base_currency -> CurrencyCode,
        rates_json -> Jsonb,
        fetched_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    expenses (id) {
        id -> Int4,
        name -> Text,
        amount -> Int4,
        currency -> CurrencyCode,
        date -> Date,
        scheduled_date -> Nullable<Date>,
        recurring_id -> Nullable<Int4>,
        planned_expense_id -> Nullable<Int4>,
        budget_id -> Nullable<Int4>,
        amount_overridden -> Bool,
        is_subscription -> Bool,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    expense_tags (expense_id, tag_id) {
        expense_id -> Int4,
        tag_id -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;
    use super::sql_types::IncomeSource;

    income (id) {
        id -> Int4,
        name -> Text,
        amount -> Int4,
        currency -> CurrencyCode,
        source -> IncomeSource,
        date -> Date,
        schedule_id -> Nullable<Int4>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;
    use super::sql_types::PayFrequency;

    income_pay_schedules (id) {
        id -> Int4,
        name -> Text,
        anchor_date -> Date,
        frequency -> PayFrequency,
        amount -> Int4,
        currency -> CurrencyCode,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    planned_expenses (id) {
        id -> Int4,
        name -> Text,
        date -> Date,
        amount -> Int4,
        currency -> CurrencyCode,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    planned_expense_tags (planned_expense_id, tag_id) {
        planned_expense_id -> Int4,
        tag_id -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;
    use super::sql_types::PayFrequency;

    recurring_expenses (id) {
        id -> Int4,
        name -> Text,
        anchor_date -> Date,
        frequency -> PayFrequency,
        amount -> Int4,
        currency -> CurrencyCode,
        is_subscription -> Bool,
        last_payment_date -> Nullable<Date>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    recurring_expense_tags (recurring_expense_id, tag_id) {
        recurring_expense_id -> Int4,
        tag_id -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    savings (id) {
        id -> Int4,
        name -> Text,
        amount -> Int4,
        currency -> CurrencyCode,
        note -> Nullable<Text>,
        date -> Date,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    tags (id) {
        id -> Int4,
        name -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    user_settings (id) {
        id -> Int4,
        display_currency -> CurrencyCode,
        primary_schedule_id -> Nullable<Int4>,
        projection_initial_free_money -> Int4,
        projection_start_date -> Nullable<Date>,
        updated_at -> Timestamptz,
    }
}

diesel::joinable!(budget_tags -> budgets (budget_id));
diesel::joinable!(budget_tags -> tags (tag_id));
diesel::joinable!(expense_tags -> expenses (expense_id));
diesel::joinable!(expense_tags -> tags (tag_id));
diesel::joinable!(expenses -> budgets (budget_id));
diesel::joinable!(expenses -> planned_expenses (planned_expense_id));
diesel::joinable!(expenses -> recurring_expenses (recurring_id));
diesel::joinable!(income -> income_pay_schedules (schedule_id));
diesel::joinable!(planned_expense_tags -> planned_expenses (planned_expense_id));
diesel::joinable!(planned_expense_tags -> tags (tag_id));
diesel::joinable!(recurring_expense_tags -> recurring_expenses (recurring_expense_id));
diesel::joinable!(recurring_expense_tags -> tags (tag_id));
diesel::joinable!(user_settings -> income_pay_schedules (primary_schedule_id));

diesel::allow_tables_to_appear_in_same_query!(
    budget_tags,
    budgets,
    exchange_rate_snapshots,
    expense_tags,
    expenses,
    income,
    income_pay_schedules,
    planned_expense_tags,
    planned_expenses,
    recurring_expense_tags,
    recurring_expenses,
    savings,
    tags,
    user_settings,
);
