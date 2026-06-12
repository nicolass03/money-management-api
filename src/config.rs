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
    pub cron_secret: Option<String>,
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

        let cron_secret = env::var("CRON_SECRET")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Ok(Self {
            host,
            port,
            database_url,
            supabase_url,
            cors_origins,
            request_timeout: Duration::from_secs(timeout_secs),
            cron_secret,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|error| format!("invalid HOST/PORT: {error}"))
    }
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
