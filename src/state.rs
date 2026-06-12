use std::sync::Arc;

use dashmap::DashSet;
use diesel_async::pooled_connection::bb8::Pool;
use diesel_async::pooled_connection::{
    AsyncDieselConnectionManager, ManagerConfig, RecyclingMethod,
};
use diesel_async::AsyncPgConnection;
use uuid::Uuid;

use crate::auth::jwt::JwtValidator;
use crate::cache::{UserDataCache, UserDataLoader};
use crate::config::Config;
use crate::rate_limit::{auth_failure_limiter, force_refresh_limiter, IpRateLimiter, UserRateLimiter};
use crate::repos::connection;

pub type DbPool = Pool<AsyncPgConnection>;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: DbPool,
    pub jwt_validator: JwtValidator,
    pub force_refresh_limiter: Arc<UserRateLimiter>,
    pub auth_failure_limiter: Arc<IpRateLimiter>,
    pub rate_limit_enabled: bool,
    pub cache: Arc<UserDataCache>,
    pub loader: UserDataLoader,
    pub known_users: Arc<DashSet<Uuid>>,
}

impl AppState {
    pub async fn new(config: &Config) -> Result<Self, String> {
        let mut manager_config = ManagerConfig::<AsyncPgConnection>::default();
        manager_config.recycling_method = RecyclingMethod::CustomFunction(Box::new(|conn| {
            Box::pin(async move {
                connection::reset_rls_context(conn).await.map_err(|error| {
                    diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        error.to_string(),
                    )))
                })
            })
        }));
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(
            &config.database_url,
            manager_config,
        );
        let db_pool = Pool::builder()
            .max_size(config.db_pool_max_size)
            .build(manager)
            .await
            .map_err(|error| format!("failed to create database pool: {error}"))?;

        let jwt_validator = JwtValidator::new(&config.supabase_url);
        jwt_validator.init().await?;

        let cache = Arc::new(UserDataCache::new(
            config.cache_enabled,
            config.cache_max_entries,
        ));
        let loader = UserDataLoader::new(db_pool.clone(), cache.clone());

        Ok(Self {
            db_pool,
            jwt_validator,
            force_refresh_limiter: force_refresh_limiter(),
            auth_failure_limiter: auth_failure_limiter(),
            rate_limit_enabled: config.rate_limit_enabled,
            cache,
            loader,
            known_users: Arc::new(DashSet::new()),
        })
    }
}
