use std::collections::HashMap;

use chrono::Utc;
use diesel::dsl::sql;
use diesel::prelude::*;
use diesel::sql_types::BigInt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{BudgetRow, CurrencyCode};
use crate::repos::{connection, expenses, settings, tags};
use crate::schema::{budgets, expenses as expenses_table};
use crate::state::DbPool;

async fn spent_by_budget_ids(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
    budget_ids: &[Uuid],
) -> Result<HashMap<Uuid, i32>, ApiError> {
    let mut map = HashMap::new();
    if budget_ids.is_empty() {
        return Ok(map);
    }

    let rows: Vec<(Option<Uuid>, i64)> = expenses_table::table
        .filter(expenses_table::user_id.eq(user_id))
        .filter(expenses_table::budget_id.eq_any(budget_ids))
        .group_by(expenses_table::budget_id)
        .select((
            expenses_table::budget_id,
            sql::<BigInt>("coalesce(sum(amount), 0)"),
        ))
        .load(conn)
        .await?;

    for (budget_id, spent) in rows {
        if let Some(id) = budget_id {
            map.insert(id, i32::try_from(spent).unwrap_or(0));
        }
    }
    Ok(map)
}

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<BudgetRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_all_with_conn(&mut conn, user_id).await
}

pub async fn list_all_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<BudgetRow>, ApiError> {
    budgets::table
        .filter(budgets::user_id.eq(user_id))
        .order(budgets::created_at.desc())
        .select(BudgetRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_with_tags_and_spent(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<(BudgetRow, Vec<String>, i32)>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_with_tags_and_spent_with_conn(&mut conn, user_id).await
}

pub async fn list_with_tags_and_spent_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<(BudgetRow, Vec<String>, i32)>, ApiError> {
    let rows = list_all_with_conn(conn, user_id).await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_budgets(conn, user_id, &ids).await?;
    let spent_map = spent_by_budget_ids(conn, user_id, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
            let spent = spent_map.get(&row.id).copied().unwrap_or(0);
            (row, tags, spent)
        })
        .collect())
}

pub async fn find_by_id(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<BudgetRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    budgets::table
        .filter(budgets::user_id.eq(user_id))
        .filter(budgets::id.eq(id))
        .select(BudgetRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn find_with_tags_and_spent(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<(BudgetRow, Vec<String>, i32)>, ApiError> {
    let Some(row) = find_by_id(pool, user_id, id).await? else {
        return Ok(None);
    };
    let mut conn = connection::user_connection(pool, user_id).await?;
    let tag_map = tags::tags_for_budgets(&mut conn, user_id, &[id]).await?;
    let spent_map = spent_by_budget_ids(&mut conn, user_id, &[id]).await?;
    Ok(Some((
        row,
        tag_map.get(&id).cloned().unwrap_or_default(),
        spent_map.get(&id).copied().unwrap_or(0),
    )))
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
    tag_names: &[String],
) -> Result<BudgetRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let budget = diesel::insert_into(budgets::table)
                .values((
                    budgets::user_id.eq(user_id),
                    budgets::name.eq(name),
                    budgets::amount.eq(amount),
                    budgets::currency.eq(currency),
                    budgets::start_date.eq(start_date),
                    budgets::end_date.eq(end_date),
                    budgets::created_at.eq(now),
                    budgets::updated_at.eq(now),
                ))
                .returning(BudgetRow::as_returning())
                .get_result(conn)
                .await?;
            tags::set_budget_tags(conn, user_id, budget.id, tag_names).await?;
            settings::bump_cache_revision(conn, user_id).await?;
            Ok::<BudgetRow, diesel::result::Error>(budget)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn update(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
    tag_names: &[String],
) -> Result<Option<BudgetRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let budget = diesel::update(
                budgets::table
                    .filter(budgets::user_id.eq(user_id))
                    .filter(budgets::id.eq(id)),
            )
            .set((
                budgets::name.eq(name),
                budgets::amount.eq(amount),
                budgets::currency.eq(currency),
                budgets::start_date.eq(start_date),
                budgets::end_date.eq(end_date),
                budgets::updated_at.eq(now),
            ))
            .returning(BudgetRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if let Some(ref row) = budget {
                tags::set_budget_tags(conn, user_id, row.id, tag_names).await?;
            }
            if budget.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<BudgetRow>, diesel::result::Error>(budget)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let count = expenses::count_by_budget(pool, user_id, id).await?;
    if count > 0 {
        return Err(ApiError::BadRequest(
            "cannot delete budget with recorded expenses".into(),
        ));
    }
    let mut conn = connection::user_connection(pool, user_id).await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            let deleted = diesel::delete(
                budgets::table
                    .filter(budgets::user_id.eq(user_id))
                    .filter(budgets::id.eq(id)),
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

pub async fn create_budget_expense(
    pool: &DbPool,
    user_id: Uuid,
    budget_id: Uuid,
    name: &str,
    amount: i32,
    currency: CurrencyCode,
    date: chrono::NaiveDate,
) -> Result<crate::models::ExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    let result = conn
        .transaction(|conn| {
            Box::pin(async move {
                let budget: BudgetRow = budgets::table
                    .filter(budgets::user_id.eq(user_id))
                    .filter(budgets::id.eq(budget_id))
                    .for_update()
                    .select(BudgetRow::as_select())
                    .first(conn)
                    .await?;

                let spent: i64 = expenses_table::table
                    .filter(expenses_table::user_id.eq(user_id))
                    .filter(expenses_table::budget_id.eq(budget_id))
                    .select(sql::<BigInt>("coalesce(sum(amount), 0)"))
                    .first(conn)
                    .await?;

                let spent = i32::try_from(spent).unwrap_or(i32::MAX);
                if amount > budget.amount.saturating_sub(spent) {
                    return Err(diesel::result::Error::SerializationError(
                        "amount exceeds remaining budget".into(),
                    ));
                }

                let expense = expenses::insert_expense(
                    conn,
                    user_id,
                    name,
                    amount,
                    currency,
                    date,
                    None,
                    None,
                    None,
                    Some(budget_id),
                    false,
                    false,
                    now,
                )
                .await?;
                tags::copy_budget_tags_to_expense(conn, budget_id, expense.id).await?;
                settings::bump_cache_revision(conn, user_id).await?;
                Ok::<crate::models::ExpenseRow, diesel::result::Error>(expense)
            })
        })
        .await;

    match result {
        Ok(expense) => Ok(expense),
        Err(diesel::result::Error::SerializationError(message))
            if message.to_string() == "amount exceeds remaining budget" =>
        {
            Err(ApiError::BadRequest("amount exceeds remaining budget".into()))
        }
        Err(error) => Err(ApiError::from(error)),
    }
}

pub async fn delete_budget_expense(
    pool: &DbPool,
    user_id: Uuid,
    budget_id: Uuid,
    expense_id: Uuid,
) -> Result<bool, ApiError> {
    let existing = expenses::find_by_id(pool, user_id, expense_id).await?;
    let Some(expense) = existing else {
        return Ok(false);
    };
    if expense.budget_id != Some(budget_id) {
        return Ok(false);
    }
    expenses::delete(pool, user_id, expense_id).await?;
    Ok(true)
}
