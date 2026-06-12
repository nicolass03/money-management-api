use diesel_async::pooled_connection::bb8::Pool;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncPgConnection;

use crate::auth::jwt::JwtValidator;
use crate::config::Config;

pub type DbPool = Pool<AsyncPgConnection>;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    pub jwt_validator: JwtValidator,
    pub cron_secret: Option<String>,
}

impl AppState {
    pub async fn new(config: &Config) -> Result<Self, String> {
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
        let db_pool = Pool::builder()
            .build(manager)
            .await
            .map_err(|error| format!("failed to create database pool: {error}"))?;

        let jwt_validator = JwtValidator::new(&config.supabase_url);
        jwt_validator.init().await?;

        Ok(Self {
            db_pool,
            jwt_validator,
            cron_secret: config.cron_secret.clone(),
        })
    }
}
