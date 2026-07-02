// @generated automatically by Diesel CLI.

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

    accounts (id) {
        id -> Uuid,
        user_id -> Uuid,
        name -> Nullable<Text>,
        currency -> CurrencyCode,
        initial_amount -> Int4,
        archived_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    budget_tags (budget_id, tag_id) {
        budget_id -> Uuid,
        tag_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    budgets (id) {
        name -> Text,
        amount -> Int4,
        currency -> CurrencyCode,
        start_date -> Nullable<Date>,
        end_date -> Nullable<Date>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        user_id -> Uuid,
        id -> Uuid,
        completed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    exchange_rate_snapshots (id) {
        base_currency -> CurrencyCode,
        rates_json -> Jsonb,
        fetched_at -> Timestamptz,
        id -> Uuid,
    }
}

diesel::table! {
    expense_tags (expense_id, tag_id) {
        expense_id -> Uuid,
        tag_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    expenses (id) {
        name -> Text,
        amount -> Int4,
        date -> Date,
        created_at -> Timestamptz,
        currency -> CurrencyCode,
        amount_overridden -> Bool,
        is_subscription -> Bool,
        scheduled_date -> Nullable<Date>,
        user_id -> Uuid,
        id -> Uuid,
        recurring_id -> Nullable<Uuid>,
        planned_expense_id -> Nullable<Uuid>,
        budget_id -> Nullable<Uuid>,
        account_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::IncomeSource;
    use super::sql_types::CurrencyCode;

    income (id) {
        name -> Text,
        amount -> Int4,
        source -> IncomeSource,
        date -> Date,
        created_at -> Timestamptz,
        currency -> CurrencyCode,
        user_id -> Uuid,
        id -> Uuid,
        schedule_id -> Nullable<Uuid>,
        amount_overridden -> Bool,
        deleted_at -> Nullable<Timestamptz>,
        account_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PayFrequency;
    use super::sql_types::CurrencyCode;

    income_pay_schedules (id) {
        name -> Text,
        anchor_date -> Date,
        frequency -> PayFrequency,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        amount -> Int4,
        currency -> CurrencyCode,
        user_id -> Uuid,
        id -> Uuid,
        account_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    planned_expense_tags (planned_expense_id, tag_id) {
        planned_expense_id -> Uuid,
        tag_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    planned_expenses (id) {
        name -> Text,
        date -> Date,
        amount -> Int4,
        currency -> CurrencyCode,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        user_id -> Uuid,
        id -> Uuid,
        account_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    recurring_expense_tags (recurring_expense_id, tag_id) {
        recurring_expense_id -> Uuid,
        tag_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PayFrequency;
    use super::sql_types::CurrencyCode;

    recurring_expenses (id) {
        name -> Text,
        anchor_date -> Date,
        frequency -> PayFrequency,
        amount -> Int4,
        currency -> CurrencyCode,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        is_subscription -> Bool,
        last_payment_date -> Nullable<Date>,
        user_id -> Uuid,
        id -> Uuid,
        cancel_reminder_enabled -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    savings (id) {
        name -> Text,
        amount -> Int4,
        note -> Nullable<Text>,
        date -> Date,
        created_at -> Timestamptz,
        currency -> CurrencyCode,
        user_id -> Uuid,
        id -> Uuid,
    }
}

diesel::table! {
    subscription_reminders (id) {
        id -> Uuid,
        user_id -> Uuid,
        recurring_expense_id -> Uuid,
        kind -> Text,
        charge_date -> Date,
        created_at -> Timestamptz,
        dismissed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    tags (id) {
        name -> Text,
        created_at -> Timestamptz,
        user_id -> Uuid,
        id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CurrencyCode;

    user_settings (user_id) {
        display_currency -> CurrencyCode,
        updated_at -> Timestamptz,
        projection_initial_free_money -> Int4,
        projection_start_date -> Nullable<Date>,
        user_id -> Uuid,
        primary_schedule_id -> Nullable<Uuid>,
        cache_revision -> Int8,
        extra_spent_limit -> Nullable<Int4>,
        language -> Text,
        theme -> Text,
    }
}

diesel::table! {
    users (id) {
        id -> Uuid,
        email -> Nullable<Text>,
        created_at -> Timestamptz,
        onboarding_completed_at -> Nullable<Timestamptz>,
    }
}

diesel::joinable!(accounts -> users (user_id));
diesel::joinable!(budget_tags -> budgets (budget_id));
diesel::joinable!(budget_tags -> tags (tag_id));
diesel::joinable!(budgets -> users (user_id));
diesel::joinable!(expense_tags -> expenses (expense_id));
diesel::joinable!(expense_tags -> tags (tag_id));
diesel::joinable!(expenses -> accounts (account_id));
diesel::joinable!(expenses -> budgets (budget_id));
diesel::joinable!(expenses -> planned_expenses (planned_expense_id));
diesel::joinable!(expenses -> recurring_expenses (recurring_id));
diesel::joinable!(expenses -> users (user_id));
diesel::joinable!(income -> accounts (account_id));
diesel::joinable!(income -> income_pay_schedules (schedule_id));
diesel::joinable!(income -> users (user_id));
diesel::joinable!(income_pay_schedules -> accounts (account_id));
diesel::joinable!(income_pay_schedules -> users (user_id));
diesel::joinable!(planned_expense_tags -> planned_expenses (planned_expense_id));
diesel::joinable!(planned_expense_tags -> tags (tag_id));
diesel::joinable!(planned_expenses -> accounts (account_id));
diesel::joinable!(planned_expenses -> users (user_id));
diesel::joinable!(recurring_expense_tags -> recurring_expenses (recurring_expense_id));
diesel::joinable!(recurring_expense_tags -> tags (tag_id));
diesel::joinable!(recurring_expenses -> users (user_id));
diesel::joinable!(savings -> users (user_id));
diesel::joinable!(subscription_reminders -> recurring_expenses (recurring_expense_id));
diesel::joinable!(subscription_reminders -> users (user_id));
diesel::joinable!(tags -> users (user_id));
diesel::joinable!(user_settings -> income_pay_schedules (primary_schedule_id));
diesel::joinable!(user_settings -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    accounts,
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
    subscription_reminders,
    tags,
    user_settings,
    users,
);
