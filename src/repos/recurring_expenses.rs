use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::{CurrencyCode, PayFrequency, RecurringExpenseRow};
use crate::repos::tags;
use crate::schema::{expenses, recurring_expenses};
use crate::state::DbPool;

pub async fn list_all(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<RecurringExpenseRow>, ApiError> {
    let mut conn = pool.get().await?;
    recurring_expenses::table
        .filter(recurring_expenses::user_id.eq(user_id))
        .order(recurring_expenses::name.asc())
        .select(RecurringExpenseRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}

pub async fn list_with_tags(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Vec<(RecurringExpenseRow, Vec<String>)>, ApiError> {
    let rows = list_all(pool, user_id).await?;
    let mut conn = pool.get().await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let tag_map = tags::tags_for_recurring(&mut conn, user_id, &ids).await?;
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
) -> Result<Option<RecurringExpenseRow>, ApiError> {
    let mut conn = pool.get().await?;
    recurring_expenses::table
        .filter(recurring_expenses::user_id.eq(user_id))
        .filter(recurring_expenses::id.eq(id))
        .select(RecurringExpenseRow::as_select())
        .first(&mut conn)
        .await
        .optional()
        .map_err(ApiError::from)
}

pub async fn find_with_tags(
    pool: &DbPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<(RecurringExpenseRow, Vec<String>)>, ApiError> {
    let Some(row) = find_by_id(pool, user_id, id).await? else {
        return Ok(None);
    };
    let mut conn = pool.get().await?;
    let tag_map = tags::tags_for_recurring(&mut conn, user_id, &[id]).await?;
    Ok(Some((row, tag_map.get(&id).cloned().unwrap_or_default())))
}

pub async fn create(
    pool: &DbPool,
    user_id: Uuid,
    name: &str,
    anchor_date: chrono::NaiveDate,
    frequency: PayFrequency,
    amount: i32,
    currency: CurrencyCode,
    tag_names: &[String],
    is_subscription: bool,
    last_payment_date: Option<chrono::NaiveDate>,
) -> Result<RecurringExpenseRow, ApiError> {
    let mut conn = pool.get().await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let recurring = diesel::insert_into(recurring_expenses::table)
                .values((
                    recurring_expenses::user_id.eq(user_id),
                    recurring_expenses::name.eq(name),
                    recurring_expenses::anchor_date.eq(anchor_date),
                    recurring_expenses::frequency.eq(frequency),
                    recurring_expenses::amount.eq(amount),
                    recurring_expenses::currency.eq(currency),
                    recurring_expenses::is_subscription.eq(is_subscription),
                    recurring_expenses::last_payment_date.eq(last_payment_date),
                    recurring_expenses::created_at.eq(now),
                    recurring_expenses::updated_at.eq(now),
                ))
                .returning(RecurringExpenseRow::as_returning())
                .get_result(conn)
                .await?;
            tags::set_recurring_expense_tags(conn, user_id, recurring.id, tag_names).await?;
            Ok::<RecurringExpenseRow, diesel::result::Error>(recurring)
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
    anchor_date: chrono::NaiveDate,
    frequency: PayFrequency,
    amount: i32,
    currency: CurrencyCode,
    tag_names: &[String],
    is_subscription: bool,
    last_payment_date: Option<chrono::NaiveDate>,
) -> Result<Option<RecurringExpenseRow>, ApiError> {
    let mut conn = pool.get().await?;
    let now = Utc::now();
    conn.transaction(|conn| {
        Box::pin(async move {
            let recurring = diesel::update(
                recurring_expenses::table
                    .filter(recurring_expenses::user_id.eq(user_id))
                    .filter(recurring_expenses::id.eq(id)),
            )
            .set((
                recurring_expenses::name.eq(name),
                recurring_expenses::anchor_date.eq(anchor_date),
                recurring_expenses::frequency.eq(frequency),
                recurring_expenses::amount.eq(amount),
                recurring_expenses::currency.eq(currency),
                recurring_expenses::is_subscription.eq(is_subscription),
                recurring_expenses::last_payment_date.eq(last_payment_date),
                recurring_expenses::updated_at.eq(now),
            ))
            .returning(RecurringExpenseRow::as_returning())
            .get_result(conn)
            .await
            .optional()?;
            if let Some(ref row) = recurring {
                tags::set_recurring_expense_tags(conn, user_id, row.id, tag_names).await?;
            }
            Ok::<Option<RecurringExpenseRow>, diesel::result::Error>(recurring)
        })
    })
    .await
    .map_err(ApiError::from)
}

pub async fn delete(pool: &DbPool, user_id: Uuid, id: Uuid) -> Result<(), ApiError> {
    let mut conn = pool.get().await?;
    conn.transaction(|conn| {
        Box::pin(async move {
            diesel::delete(
                expenses::table
                    .filter(expenses::user_id.eq(user_id))
                    .filter(expenses::recurring_id.eq(id)),
            )
            .execute(conn)
            .await?;
            diesel::delete(
                recurring_expenses::table
                    .filter(recurring_expenses::user_id.eq(user_id))
                    .filter(recurring_expenses::id.eq(id)),
            )
            .execute(conn)
            .await?;
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(ApiError::from)
}
