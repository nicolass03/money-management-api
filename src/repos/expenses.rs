use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use diesel::dsl::sql;
use diesel::prelude::*;
use diesel::sql_types::BigInt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, ExpenseRow};
use crate::repos::{connection, settings, tags};
use crate::schema::expenses;
use crate::state::DbPool;
use diesel_async::AsyncPgConnection;

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<ExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_all_with_conn(&mut conn, user_id).await
}

pub async fn list_all_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<ExpenseRow>, ApiError> {
    expenses::table
        .filter(expenses::user_id.eq(user_id))
        .order(expenses::created_at.desc())
        .select(ExpenseRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_with_tags(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<(ExpenseRow, Vec<String>)>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_with_tags_with_conn(&mut conn, user_id).await
}

pub async fn list_with_tags_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<(ExpenseRow, Vec<String>)>, ApiError> {
    let rows = list_all_with_conn(conn, user_id).await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_expenses(conn, user_id, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
            (row, tags)
        })
        .collect())
}

pub async fn list_with_tags_in_range(
    pool: &DbPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<(ExpenseRow, Vec<String>)>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_with_tags_in_range_with_conn(&mut conn, user_id, from, to).await
}

pub async fn list_with_tags_in_range_with_conn(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<(ExpenseRow, Vec<String>)>, ApiError> {
    let rows = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::date.ge(from))
        .filter(expenses::date.le(to))
        .order(expenses::created_at.desc())
        .select(ExpenseRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_expenses(conn, user_id, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
            (row, tags)
        })
        .collect())
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<ExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::id.eq(id))
        .select(ExpenseRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn find_with_tags(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<(ExpenseRow, Vec<String>)>, ApiError> {
    let Some(row) = find_by_id(pool, user_id, id).await? else {
        return Ok(None);
    };
    let mut conn = connection::user_connection(pool, user_id).await?;
    let tag_map = tags::tags_for_expenses(&mut conn, user_id, &[id]).await?;
    Ok(Some((row, tag_map.get(&id).cloned().unwrap_or_default())))
}

/// Column values for a new expense row. Grouping them into a named struct removes the transposition
/// hazard of a 13-argument call — several `Option<Uuid>` and two `bool`s sit next to each other, and
/// positionally swapping any two same-typed values would compile silently.
pub struct NewExpense<'a> {
    pub user_id: Uuid,
    pub name: &'a str,
    pub amount: i32,
    pub currency: CurrencyCode,
    pub date: NaiveDate,
    pub scheduled_date: Option<NaiveDate>,
    pub recurring_id: Option<Uuid>,
    pub planned_expense_id: Option<Uuid>,
    pub budget_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub amount_overridden: bool,
    pub is_subscription: bool,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_expense(
    conn: &mut AsyncPgConnection,
    new: NewExpense<'_>,
) -> Result<ExpenseRow, diesel::result::Error> {
    diesel::insert_into(expenses::table)
        .values((
            expenses::user_id.eq(new.user_id),
            expenses::name.eq(new.name),
            expenses::amount.eq(new.amount),
            expenses::currency.eq(new.currency),
            expenses::date.eq(new.date),
            expenses::scheduled_date.eq(new.scheduled_date),
            expenses::recurring_id.eq(new.recurring_id),
            expenses::planned_expense_id.eq(new.planned_expense_id),
            expenses::budget_id.eq(new.budget_id),
            expenses::account_id.eq(new.account_id),
            expenses::amount_overridden.eq(new.amount_overridden),
            expenses::is_subscription.eq(new.is_subscription),
            expenses::created_at.eq(new.created_at),
        ))
        .returning(ExpenseRow::as_returning())
        .get_result(conn)
        .await
}

pub async fn create_manual(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    date: NaiveDate,
    tag_names: &[String],
    is_subscription: bool,
    account_id: Option<Uuid>,
) -> Result<ExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let expense = insert_expense(
                conn,
                NewExpense {
                    user_id,
                    name,
                    amount,
                    currency,
                    date,
                    scheduled_date: None,
                    recurring_id: None,
                    planned_expense_id: None,
                    budget_id: None,
                    account_id,
                    amount_overridden: false,
                    is_subscription,
                    created_at: now,
                },
            )
            .await?;
            tags::set_expense_tags(conn, user_id, expense.id, tag_names).await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<ExpenseRow, diesel::result::Error>(expense)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn update_amount(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    amount: i32,
) -> Result<Option<ExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let expense = diesel::update(
                expenses::table
                    .filter(expenses::user_id.eq(user_id))
                    .filter(expenses::id.eq(id)),
            )
            .set((
                expenses::amount.eq(amount),
                expenses::amount_overridden.eq(true),
            ))
            .returning(ExpenseRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if expense.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<ExpenseRow>, diesel::result::Error>(expense)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let deleted = diesel::delete(
                expenses::table
                    .filter(expenses::user_id.eq(user_id))
                    .filter(expenses::id.eq(id)),
            )
            .execute(conn)
            .await?;
            if deleted > 0 {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn find_by_recurring_and_due_date(
    pool: &DbPool,
    user_id: Uuid,
    recurring_id: Uuid,
    due_date: NaiveDate,
) -> Result<Option<ExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::recurring_id.eq(recurring_id))
        .filter(
            expenses::scheduled_date
                .eq(due_date)
                .or(expenses::scheduled_date
                    .is_null()
                    .and(expenses::date.eq(due_date))),
        )
        .select(ExpenseRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn find_by_planned_id(
    pool: &DbPool,
    user_id: Uuid,
    planned_id: Uuid,
) -> Result<Option<ExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::planned_expense_id.eq(planned_id))
        .select(ExpenseRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

/// Ids of planned (one-time) expenses that already have a recorded payment.
pub async fn list_paid_planned_ids(pool: &DbPool, user_id: Uuid) -> Result<HashSet<Uuid>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let rows: Vec<Option<Uuid>> = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::planned_expense_id.is_not_null())
        .select(expenses::planned_expense_id)
        .load(&mut conn)
        .await?;
    Ok(rows.into_iter().flatten().collect())
}

pub async fn get_materialized_recurring_ids_for_due_date(
    pool: &DbPool,
    user_id: Uuid,
    date: &str,
) -> Result<HashSet<Uuid>, ApiError> {
    let due_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest("invalid date".into()))?;
    let mut conn = connection::user_connection(pool, user_id).await?;
    let rows: Vec<Option<Uuid>> = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::recurring_id.is_not_null())
        .filter(
            expenses::scheduled_date
                .eq(due_date)
                .or(expenses::scheduled_date
                    .is_null()
                    .and(expenses::date.eq(due_date))),
        )
        .select(expenses::recurring_id)
        .load(&mut conn)
        .await?;
    Ok(rows.into_iter().flatten().collect())
}

pub async fn create_early_paid(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    date: NaiveDate,
    scheduled_date: Option<NaiveDate>,
    recurring_id: Option<Uuid>,
    planned_expense_id: Option<Uuid>,
    account_id: Option<Uuid>,
    amount_overridden: bool,
    is_subscription: bool,
) -> Result<ExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let expense = insert_expense(
                conn,
                NewExpense {
                    user_id,
                    name,
                    amount,
                    currency,
                    date,
                    scheduled_date,
                    recurring_id,
                    planned_expense_id,
                    budget_id: None,
                    account_id,
                    amount_overridden,
                    is_subscription,
                    created_at: now,
                },
            )
            .await?;
            if let Some(recurring_id) = recurring_id {
                tags::copy_recurring_tags_to_expense(conn, recurring_id, expense.id).await?;
            } else if let Some(planned_id) = planned_expense_id {
                tags::copy_planned_tags_to_expense(conn, planned_id, expense.id).await?;
            }
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<ExpenseRow, diesel::result::Error>(expense)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn list_by_budget(
    pool: &DbPool,
    user_id: Uuid,
    budget_id: Uuid,
) -> Result<Vec<(ExpenseRow, Vec<String>)>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let rows = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::budget_id.eq(budget_id))
        .order(expenses::created_at.desc())
        .select(ExpenseRow::as_select())
        .load(&mut conn)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_expenses(&mut conn, user_id, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
            (row, tags)
        })
        .collect())
}

pub async fn count_by_budget(
    pool: &DbPool,
    user_id: Uuid,
    budget_id: Uuid,
) -> Result<i64, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let count: i64 = expenses::table
        .filter(expenses::user_id.eq(user_id))
        .filter(expenses::budget_id.eq(budget_id))
        .select(sql::<BigInt>("count(*)"))
        .get_result(&mut conn)
        .await?;
    Ok(count)
}
