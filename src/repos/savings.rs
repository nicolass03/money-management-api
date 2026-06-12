use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::SavingRow;
use crate::repos::connection;
use crate::schema::savings;
use crate::state::DbPool;

pub async fn list_all(pool: &DbPool, user_id: Uuid) -> Result<Vec<SavingRow>, ApiError> {
    let mut conn = connection::user_connection(pool, user_id).await?;
    savings::table
        .filter(savings::user_id.eq(user_id))
        .order(savings::date.desc())
        .select(SavingRow::as_select())
        .load(&mut conn)
        .await
        .map_err(ApiError::from)
}
