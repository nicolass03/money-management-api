use std::env;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub supabase_url: String,
    pub cors_origins: Vec<String>,
    pub request_timeout: Duration,
    pub enable_internal_cron: bool,
    pub daily_expenses_hour: u8,
    pub rate_limit_enabled: bool,
    pub cache_enabled: bool,
    pub cache_max_entries: u64,
    pub db_pool_max_size: u32,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .map_err(|_| "PORT must be a valid u16".to_string())?;

        let database_url = required_env("DATABASE_URL")?;
        let supabase_url = required_env("SUPABASE_URL")?;

        let cors_origins = env::var("CORS_ORIGIN")
            .unwrap_or_else(|_| "http://localhost:3000".to_string())
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();

        let timeout_secs = env::var("REQUEST_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .map_err(|_| "REQUEST_TIMEOUT_SECS must be a valid u64".to_string())?;

        let enable_internal_cron = env::var("ENABLE_INTERNAL_CRON")
            .map(|value| parse_bool(&value))
            .unwrap_or(true);

        let daily_expenses_hour = env::var("DAILY_EXPENSES_HOUR")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u8>()
            .map_err(|_| "DAILY_EXPENSES_HOUR must be 0-23".to_string())?;

        if daily_expenses_hour > 23 {
            return Err("DAILY_EXPENSES_HOUR must be 0-23".to_string());
        }

        let rate_limit_enabled = env::var("RATE_LIMIT_ENABLED")
            .map(|value| parse_bool(&value))
            .unwrap_or(!cfg!(debug_assertions));

        let cache_enabled = env::var("CACHE_ENABLED")
            .map(|value| parse_bool(&value))
            .unwrap_or(true);

        let cache_max_entries = env::var("CACHE_MAX_ENTRIES")
            .unwrap_or_else(|_| "10000".to_string())
            .parse::<u64>()
            .map_err(|_| "CACHE_MAX_ENTRIES must be a valid u64".to_string())?;

        let db_pool_max_size = env::var("DB_POOL_MAX_SIZE")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .map_err(|_| "DB_POOL_MAX_SIZE must be a valid u32".to_string())?;

        Ok(Self {
            host,
            port,
            database_url,
            supabase_url,
            cors_origins,
            request_timeout: Duration::from_secs(timeout_secs),
            enable_internal_cron,
            daily_expenses_hour,
            rate_limit_enabled,
            cache_enabled,
            cache_max_entries,
            db_pool_max_size,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|error| format!("invalid HOST/PORT: {error}"))
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn required_env(key: &str) -> Result<String, String> {
    env::var(key)
        .map(|value| value.trim().to_string())
        .map_err(|_| format!("{key} is required"))
        .and_then(|value| {
            if value.is_empty() {
                Err(format!("{key} is required"))
            } else {
                Ok(value)
            }
        })
}
