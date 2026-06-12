use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PlannedExpenseRow};
use crate::repos::{connection, settings, tags};
use crate::schema::planned_expenses;
use crate::state::DbPool;

pub async fn list_all(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<PlannedExpenseRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_all_with_conn(&mut conn, user_id).await
}

pub async fn list_all_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<PlannedExpenseRow>, ApiError> {
    planned_expenses::table
        .filter(planned_expenses::user_id.eq(user_id))
        .order(planned_expenses::date.asc())
        .select(PlannedExpenseRow::as_select())
        .load(conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_with_tags(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<(PlannedExpenseRow, Vec<String>)>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    list_with_tags_with_conn(&mut conn, user_id).await
}

pub async fn list_with_tags_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: Uuid,
) -> Result<Vec<(PlannedExpenseRow, Vec<String>)>, ApiError> {
    let rows = list_all_with_conn(conn, user_id).await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_planned(conn, user_id, &ids).await?;
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
            settings::bump_cache_revision(conn, user_id).await?;
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
            if planned.is_some() {
                settings::bump_cache_revision(conn, user_id).await?;
            }
            Ok::<Option<PlannedExpenseRow>, diesel::result::Error>(planned)
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
                planned_expenses::table
                    .filter(planned_expenses::user_id.eq(user_id))
                    .filter(planned_expenses::id.eq(id)),
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
