use std::sync::Arc;

use dashmap::DashSet;
use diesel::{ConnectionError, ConnectionResult};
use diesel_async::pooled_connection::bb8::Pool;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::AsyncPgConnection;
use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use uuid::Uuid;

use crate::auth::jwt::JwtValidator;
use crate::cache::{UserDataCache, UserDataLoader};
use crate::config::Config;
use crate::rate_limit::{auth_failure_limiter, force_refresh_limiter, IpRateLimiter, UserRateLimiter};

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
        // Establish every pooled connection over TLS (rustls). Supabase requires SSL over the
        // public internet, and diesel-async's default establish path is plaintext-only — so the
        // pool is built with a custom setup that negotiates TLS via tokio-postgres-rustls.
        //
        // Connections are NOT reset on recycle: that would cost a full round-trip on every
        // checkout. Instead each checkout sets its RLS context in a single statement
        // (see `repos::connection`), so the default fast recycling is both correct and cheap.
        // Build the rustls TLS connector once and share it across every connection the pool
        // establishes (cloning the connector is cheap — it wraps an `Arc<ClientConfig>`).
        let tls = build_tls_connector()?;
        let mut manager_config = ManagerConfig::<AsyncPgConnection>::default();
        manager_config.custom_setup =
            Box::new(move |url| establish_tls_connection(url, tls.clone()));
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(
            &config.database_url,
            manager_config,
        );
        // min_idle can never exceed max_size, otherwise bb8 rejects the config.
        let min_idle = config.db_pool_min_idle.min(config.db_pool_max_size);
        let db_pool = Pool::builder()
            .max_size(config.db_pool_max_size)
            .min_idle(Some(min_idle))
            .connection_timeout(config.db_pool_connection_timeout)
            .idle_timeout(Some(config.db_pool_idle_timeout))
            .max_lifetime(Some(config.db_pool_max_lifetime))
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

/// Supabase Root 2021 CA — Supabase secures Postgres / the connection pooler with its own private
/// CA rather than a public one, so it must be trusted explicitly for full certificate verification.
const SUPABASE_ROOT_CA: &str = include_str!("../certs/supabase-root-2021-ca.pem");

/// Builds the shared rustls connector used for every database connection. Trusts the bundled
/// Supabase CA plus the public Mozilla roots, and any extra CA supplied via `DATABASE_CA_CERT`
/// (a PEM file path) for certificate rotation without a rebuild. Hostname + chain are fully
/// verified (`verify-full`); built once at startup.
fn build_tls_connector() -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    add_pem_roots(&mut roots, SUPABASE_ROOT_CA.as_bytes())
        .map_err(|error| format!("bundled Supabase CA is invalid: {error}"))?;

    if let Ok(path) = std::env::var("DATABASE_CA_CERT") {
        let pem = std::fs::read(&path)
            .map_err(|error| format!("failed to read DATABASE_CA_CERT '{path}': {error}"))?;
        add_pem_roots(&mut roots, &pem)
            .map_err(|error| format!("DATABASE_CA_CERT '{path}' is invalid: {error}"))?;
    }

    let tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| format!("failed to configure TLS protocol versions: {error}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();

    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(tls_config))
}

/// Parses every certificate in a PEM buffer into the trust store (non-certificate lines, e.g.
/// comments, are ignored by the PEM reader).
fn add_pem_roots(store: &mut rustls::RootCertStore, pem: &[u8]) -> Result<(), String> {
    let mut reader = std::io::BufReader::new(pem);
    for cert in rustls_pemfile::certs(&mut reader) {
        let cert = cert.map_err(|error| error.to_string())?;
        store.add(cert).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Opens a single Postgres connection to Supabase over TLS and adapts it into an
/// `AsyncPgConnection`. Used by the bb8 manager's `custom_setup`, so every connection the pool
/// creates (including warm `min_idle` ones) is encrypted.
fn establish_tls_connection(
    database_url: &str,
    tls: tokio_postgres_rustls::MakeRustlsConnect,
) -> BoxFuture<'static, ConnectionResult<AsyncPgConnection>> {
    let database_url = database_url.to_string();
    async move {
        let (client, connection) = tokio_postgres::connect(&database_url, tls)
            .await
            // tokio_postgres' top-level Display is just "db error"; include the source chain so
            // pool-build failures (auth, pooler limits, TLS) are actually diagnosable in logs.
            .map_err(|error| {
                let mut chain = format!("{error}");
                let mut src = std::error::Error::source(&error);
                while let Some(e) = src {
                    chain.push_str(&format!(": {e}"));
                    src = e.source();
                }
                ConnectionError::BadConnection(chain)
            })?;

        AsyncPgConnection::try_from_client_and_connection(client, connection).await
    }
    .boxed()
}
