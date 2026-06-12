use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PlannedExpenseRow};
use crate::repos::{connection, tags};
use crate::schema::planned_expenses;
use crate::state::DbPool;

pub async fn list_all(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<PlannedExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    planned_expenses::table
        .filter(planned_expenses::user_id.eq(user_id))
        .order(planned_expenses::date.asc())
        .select(PlannedExpenseRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_with_tags(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<(PlannedExpenseRow, Vec<String>)>, ApiError> {
    let rows = list_all(pool, user_id).await?;
    let mut conn = connection::user_connection(pool, user_id).await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_planned(&mut conn, user_id, &ids).await?;
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
) -> Result<Option<PlannedExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    planned_expenses::table
        .filter(planned_expenses::user_id.eq(user_id))
        .filter(planned_expenses::id.eq(id))
        .select(PlannedExpenseRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn find_with_tags(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<(PlannedExpenseRow, Vec<String>)>, ApiError> {
    let Some(row) = find_by_id(pool, user_id, id).await? else {
        return Ok(None);
    };
    let mut conn = connection::user_connection(pool, user_id).await?;
    let tag_map = tags::tags_for_planned(&mut conn, user_id, &[id]).await?;
    Ok(Some((row, tag_map.get(&id).cloned().unwrap_or_default())))
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    date: chrono::NaiveDate,
    amount: i32,
    currency: CurrencyCode,
    tag_names: &[String],
) -> Result<PlannedExpenseRow, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let planned = diesel::insert_into(planned_expenses::table)
                .values((
                    planned_expenses::user_id.eq(user_id),
                    planned_expenses::name.eq(name),
                    planned_expenses::date.eq(date),
                    planned_expenses::amount.eq(amount),
                    planned_expenses::currency.eq(currency),
                    planned_expenses::created_at.eq(now),
                    planned_expenses::updated_at.eq(now),
                ))
                .returning(PlannedExpenseRow::as_returning())
                .get_result(conn)
                .await?;
            tags::set_planned_expense_tags(conn, user_id, planned.id, tag_names).await?;
            Ok::<PlannedExpenseRow, diesel::result::Error>(planned)
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
    date: chrono::NaiveDate,
    amount: i32,
    currency: CurrencyCode,
    tag_names: &[String],
) -> Result<Option<PlannedExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let planned = diesel::update(
                planned_expenses::table
                    .filter(planned_expenses::user_id.eq(user_id))
                    .filter(planned_expenses::id.eq(id)),
            )
            .set((
                planned_expenses::name.eq(name),
                planned_expenses::date.eq(date),
                planned_expenses::amount.eq(amount),
                planned_expenses::currency.eq(currency),
                planned_expenses::updated_at.eq(now),
            ))
            .returning(PlannedExpenseRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if let Some(ref row) = planned {
                tags::set_planned_expense_tags(conn, user_id, row.id, tag_names).await?;
            }
            Ok::<Option<PlannedExpenseRow>, diesel::result::Error>(planned)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    diesel::delete(
        planned_expenses::table
            .filter(planned_expenses::user_id.eq(user_id))
            .filter(planned_expenses::id.eq(id)),
    )
    .execute(&mut conn)
    .await?;
    Ok(())
}
