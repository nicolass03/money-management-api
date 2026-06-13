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
        .order(expenses::date.desc())
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
        .order(expenses::date.desc())
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

pub async fn insert_expense(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    date: NaiveDate,
    scheduled_date: Option<NaiveDate>,
    recurring_id: Option<Uuid>,
    planned_expense_id: Option<Uuid>,
    budget_id: Option<Uuid>,
    amount_overridden: bool,
    is_subscription: bool,
    created_at: DateTime<Utc>,
) -> Result<ExpenseRow, diesel::result::Error> {
    diesel::insert_into(expenses::table)
        .values((
            expenses::user_id.eq(user_id),
            expenses::name.eq(name),
            expenses::amount.eq(amount),
            expenses::currency.eq(currency),
            expenses::date.eq(date),
            expenses::scheduled_date.eq(scheduled_date),
            expenses::recurring_id.eq(recurring_id),
            expenses::planned_expense_id.eq(planned_expense_id),
            expenses::budget_id.eq(budget_id),
            expenses::amount_overridden.eq(amount_overridden),
            expenses::is_subscription.eq(is_subscription),
            expenses::created_at.eq(created_at),
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
) -> Result<ExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let expense = insert_expense(
                conn,
                user_id,
                name,
                amount,
                currency,
                date,
                None,
                None,
                None,
                None,
                false,
                is_subscription,
                now,
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
    scheduled_date: NaiveDate,
    recurring_id: Option<Uuid>,
    planned_expense_id: Option<Uuid>,
    amount_overridden: bool,
    is_subscription: bool,
) -> Result<ExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let expense = insert_expense(
                conn,
                user_id,
                name,
                amount,
                currency,
                date,
                Some(scheduled_date),
                recurring_id,
                planned_expense_id,
                None,
                amount_overridden,
                is_subscription,
                now,
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
        .order(expenses::date.desc())
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
