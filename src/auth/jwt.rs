use std::collections::HashMap;
use std::sync::Arc;

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub sub: Uuid,
    pub email: String,
}

#[derive(Debug, Deserialize)]
struct SupabaseClaims {
    sub: String,
    email: Option<String>,
    role: Option<String>,
    #[allow(dead_code)]
    aud: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    alg: String,
    kty: String,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Clone)]
struct EcKey {
    x: String,
    y: String,
}

#[derive(Clone)]
pub struct JwtValidator {
    keys: Arc<RwLock<HashMap<String, EcKey>>>,
    fetch_url: String,
    issuer: String,
    http: reqwest::Client,
}

impl JwtValidator {
    pub fn new(supabase_url: &str) -> Self {
        let base = supabase_url.trim_end_matches('/');
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            fetch_url: format!("{base}/auth/v1/.well-known/jwks.json"),
            issuer: format!("{base}/auth/v1"),
            http: reqwest::Client::new(),
        }
    }

    pub async fn init(&self) -> Result<(), String> {
        self.refresh().await
    }

    pub async fn refresh(&self) -> Result<(), String> {
        let jwks: Jwks = self
            .http
            .get(&self.fetch_url)
            .send()
            .await
            .map_err(|error| format!("failed to fetch JWKS: {error}"))?
            .json()
            .await
            .map_err(|error| format!("failed to parse JWKS: {error}"))?;

        let mut map = HashMap::new();
        for key in jwks.keys {
            if key.kty == "EC" && key.alg == "ES256" {
                if let (Some(x), Some(y)) = (key.x, key.y) {
                    map.insert(key.kid, EcKey { x, y });
                }
            }
        }

        tracing::info!(count = map.len(), "loaded Supabase JWKS signing keys");
        *self.keys.write().await = map;
        Ok(())
    }

    pub async fn validate(&self, token: &str) -> Result<AuthUser, ()> {
        let header = decode_header(token).map_err(|error| {
            tracing::debug!(%error, "jwt header decode failed");
        })?;

        if header.alg != Algorithm::ES256 {
            tracing::debug!(alg = ?header.alg, "unsupported jwt algorithm");
            return Err(());
        }

        let kid = header.kid.ok_or_else(|| {
            tracing::debug!("jwt missing kid header");
        })?;

        match self.validate_es256(token, &kid).await {
            Ok(user) => Ok(user),
            Err(()) if !self.keys.read().await.contains_key(&kid) => {
                tracing::debug!(%kid, "jwt kid missing from cache, refreshing JWKS");
                self.refresh().await.map_err(|error| {
                    tracing::warn!(%error, "JWKS refresh failed");
                })?;
                self.validate_es256(token, &kid).await
            }
            Err(()) => Err(()),
        }
    }

    async fn validate_es256(&self, token: &str, kid: &str) -> Result<AuthUser, ()> {
        let keys = self.keys.read().await;
        let ec_key = keys.get(kid).ok_or_else(|| {
            tracing::debug!(%kid, "jwt kid not found in JWKS");
        })?;

        let decoding_key = DecodingKey::from_ec_components(&ec_key.x, &ec_key.y).map_err(|error| {
            tracing::debug!(%error, "invalid EC key in JWKS");
        })?;

        self.decode_and_check(token, &decoding_key, Algorithm::ES256)
    }

    fn decode_and_check(
        &self,
        token: &str,
        key: &DecodingKey,
        alg: Algorithm,
    ) -> Result<AuthUser, ()> {
        let mut validation = Validation::new(alg);
        validation.set_audience(&["authenticated"]);
        validation.set_issuer(&[&self.issuer]);

        let token_data = decode::<SupabaseClaims>(token, key, &validation).map_err(|error| {
            tracing::debug!(%error, "jwt validation failed");
        })?;

        let role = token_data.claims.role.as_deref().unwrap_or_default();
        if role != "authenticated" {
            tracing::debug!(%role, "jwt rejected role claim");
            return Err(());
        }

        let email = token_data.claims.email.filter(|value| !value.trim().is_empty());
        let email = email.ok_or_else(|| {
            tracing::debug!("jwt missing email claim");
        })?;

        let sub = Uuid::parse_str(&token_data.claims.sub).map_err(|error| {
            tracing::debug!(%error, "invalid jwt sub claim");
        })?;

        Ok(AuthUser { sub, email })
    }
}
