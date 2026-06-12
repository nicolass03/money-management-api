use diesel_async::pooled_connection::bb8::Pool;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncPgConnection;

use crate::config::Config;

pub type DbPool = Pool<AsyncPgConnection>;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    pub jwt_secret: String,
    pub auth_user_email: String,
}

impl AppState {
    pub async fn new(config: &Config) -> Result<Self, String> {
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
        let db_pool = Pool::builder()
            .build(manager)
            .await
            .map_err(|error| format!("failed to create database pool: {error}"))?;

        Ok(Self {
            db_pool,
            jwt_secret: config.jwt_secret.clone(),
            auth_user_email: config.auth_user_email.clone(),
        })
    }
}
