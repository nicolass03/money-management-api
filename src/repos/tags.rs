use std::collections::HashMap;

use chrono::Utc;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::schema::{budget_tags, expense_tags, planned_expense_tags, recurring_expense_tags, tags};
use diesel_async::AsyncPgConnection;

type DbResult<T> = Result<T, diesel::result::Error>;

async fn ensure_tags(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    names: &[String],
) -> DbResult<Vec<Uuid>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }

    let now = Utc::now();
    for name in names {
        diesel::insert_into(tags::table)
            .values((
                tags::user_id.eq(user_id),
                tags::name.eq(name),
                tags::created_at.eq(now),
            ))
            .on_conflict((tags::user_id, tags::name))
            .do_nothing()
            .execute(conn)
            .await?;
    }

    let rows: Vec<(Uuid, String)> = tags::table
        .filter(tags::user_id.eq(user_id))
        .filter(tags::name.eq_any(names))
        .select((tags::id, tags::name))
        .load(conn)
        .await?;

    let name_to_id: HashMap<String, Uuid> = rows.into_iter().map(|(id, name)| (name, id)).collect();
    names
        .iter()
        .map(|name| {
            name_to_id
                .get(name)
                .copied()
                .ok_or_else(|| {
                    diesel::result::Error::SerializationError(
                        format!("tag not found after upsert: {name}").into(),
                    )
                })
        })
        .collect()
}

pub async fn set_expense_tags(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    expense_id: Uuid,
    tag_names: &[String],
) -> DbResult<()> {
    diesel::delete(expense_tags::table.filter(expense_tags::expense_id.eq(expense_id)))
        .execute(conn)
        .await?;
    let tag_ids = ensure_tags(conn, user_id, tag_names).await?;
    if tag_ids.is_empty() {
        return Ok(());
    }
    for tag_id in tag_ids {
        diesel::insert_into(expense_tags::table)
            .values((expense_tags::expense_id.eq(expense_id), expense_tags::tag_id.eq(tag_id)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn set_recurring_expense_tags(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    recurring_expense_id: Uuid,
    tag_names: &[String],
) -> DbResult<()> {
    diesel::delete(
        recurring_expense_tags::table
            .filter(recurring_expense_tags::recurring_expense_id.eq(recurring_expense_id)),
    )
    .execute(conn)
    .await?;
    let tag_ids = ensure_tags(conn, user_id, tag_names).await?;
    for tag_id in tag_ids {
        diesel::insert_into(recurring_expense_tags::table)
            .values((
                recurring_expense_tags::recurring_expense_id.eq(recurring_expense_id),
                recurring_expense_tags::tag_id.eq(tag_id),
            ))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn set_planned_expense_tags(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    planned_expense_id: Uuid,
    tag_names: &[String],
) -> DbResult<()> {
    diesel::delete(
        planned_expense_tags::table
            .filter(planned_expense_tags::planned_expense_id.eq(planned_expense_id)),
    )
    .execute(conn)
    .await?;
    let tag_ids = ensure_tags(conn, user_id, tag_names).await?;
    for tag_id in tag_ids {
        diesel::insert_into(planned_expense_tags::table)
            .values((
                planned_expense_tags::planned_expense_id.eq(planned_expense_id),
                planned_expense_tags::tag_id.eq(tag_id),
            ))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn set_budget_tags(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    budget_id: Uuid,
    tag_names: &[String],
) -> DbResult<()> {
    diesel::delete(budget_tags::table.filter(budget_tags::budget_id.eq(budget_id)))
        .execute(conn)
        .await?;
    let tag_ids = ensure_tags(conn, user_id, tag_names).await?;
    for tag_id in tag_ids {
        diesel::insert_into(budget_tags::table)
            .values((budget_tags::budget_id.eq(budget_id), budget_tags::tag_id.eq(tag_id)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn copy_recurring_tags_to_expense(
    conn: &mut AsyncPgConnection,
    recurring_expense_id: Uuid,
    expense_id: Uuid,
) -> DbResult<()> {
    let links: Vec<Uuid> = recurring_expense_tags::table
        .filter(recurring_expense_tags::recurring_expense_id.eq(recurring_expense_id))
        .select(recurring_expense_tags::tag_id)
        .load(conn)
        .await?;
    for tag_id in links {
        diesel::insert_into(expense_tags::table)
            .values((expense_tags::expense_id.eq(expense_id), expense_tags::tag_id.eq(tag_id)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn copy_planned_tags_to_expense(
    conn: &mut AsyncPgConnection,
    planned_expense_id: Uuid,
    expense_id: Uuid,
) -> DbResult<()> {
    let links: Vec<Uuid> = planned_expense_tags::table
        .filter(planned_expense_tags::planned_expense_id.eq(planned_expense_id))
        .select(planned_expense_tags::tag_id)
        .load(conn)
        .await?;
    for tag_id in links {
        diesel::insert_into(expense_tags::table)
            .values((expense_tags::expense_id.eq(expense_id), expense_tags::tag_id.eq(tag_id)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

pub async fn copy_budget_tags_to_expense(
    conn: &mut AsyncPgConnection,
    budget_id: Uuid,
    expense_id: Uuid,
) -> DbResult<()> {
    let links: Vec<Uuid> = budget_tags::table
        .filter(budget_tags::budget_id.eq(budget_id))
        .select(budget_tags::tag_id)
        .load(conn)
        .await?;
    for tag_id in links {
        diesel::insert_into(expense_tags::table)
            .values((expense_tags::expense_id.eq(expense_id), expense_tags::tag_id.eq(tag_id)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

async fn tag_map_for_ids(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    ids: &[Uuid],
    join_table: &str,
) -> DbResult<HashMap<Uuid, Vec<String>>> {
    let mut map: HashMap<Uuid, Vec<String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }

    match join_table {
        "expense" => {
            let rows: Vec<(Uuid, String)> = expense_tags::table
                .inner_join(tags::table)
                .filter(tags::user_id.eq(user_id))
                .filter(expense_tags::expense_id.eq_any(ids))
                .select((expense_tags::expense_id, tags::name))
                .load(conn)
                .await?;
            for (id, name) in rows {
                map.entry(id).or_default().push(name);
            }
        }
        "recurring" => {
            let rows: Vec<(Uuid, String)> = recurring_expense_tags::table
                .inner_join(tags::table)
                .filter(tags::user_id.eq(user_id))
                .filter(recurring_expense_tags::recurring_expense_id.eq_any(ids))
                .select((recurring_expense_tags::recurring_expense_id, tags::name))
                .load(conn)
                .await?;
            for (id, name) in rows {
                map.entry(id).or_default().push(name);
            }
        }
        "planned" => {
            let rows: Vec<(Uuid, String)> = planned_expense_tags::table
                .inner_join(tags::table)
                .filter(tags::user_id.eq(user_id))
                .filter(planned_expense_tags::planned_expense_id.eq_any(ids))
                .select((planned_expense_tags::planned_expense_id, tags::name))
                .load(conn)
                .await?;
            for (id, name) in rows {
                map.entry(id).or_default().push(name);
            }
        }
        "budget" => {
            let rows: Vec<(Uuid, String)> = budget_tags::table
                .inner_join(tags::table)
                .filter(tags::user_id.eq(user_id))
                .filter(budget_tags::budget_id.eq_any(ids))
                .select((budget_tags::budget_id, tags::name))
                .load(conn)
                .await?;
            for (id, name) in rows {
                map.entry(id).or_default().push(name);
            }
        }
        _ => {}
    }

    for id in ids {
        map.entry(*id).or_default();
    }
    Ok(map)
}

pub async fn tags_for_expenses(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    ids: &[Uuid],
) -> DbResult<HashMap<Uuid, Vec<String>>> {
    tag_map_for_ids(conn, user_id, ids, "expense").await
}

pub async fn tags_for_recurring(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    ids: &[Uuid],
) -> DbResult<HashMap<Uuid, Vec<String>>> {
    tag_map_for_ids(conn, user_id, ids, "recurring").await
}

pub async fn tags_for_planned(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    ids: &[Uuid],
) -> DbResult<HashMap<Uuid, Vec<String>>> {
    tag_map_for_ids(conn, user_id, ids, "planned").await
}

pub async fn tags_for_budgets(
    conn: &mut AsyncPgConnection,
    user_id: Uuid,
    ids: &[Uuid],
) -> DbResult<HashMap<Uuid, Vec<String>>> {
    tag_map_for_ids(conn, user_id, ids, "budget").await
}

pub async fn list_all_names(conn: &mut AsyncPgConnection, user_id: Uuid) -> DbResult<Vec<String>> {
    tags::table
        .filter(tags::user_id.eq(user_id))
        .select(tags::name)
        .order(tags::name.asc())
        .load(conn)
        .await
}
