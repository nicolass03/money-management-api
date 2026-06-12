use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
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
    #[allow(dead_code)]
    aud: String,
}

pub fn validate_token(token: &str, secret: &str, expected_email: &str) -> Result<AuthUser, ()> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["authenticated"]);

    let token_data = decode::<SupabaseClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|error| {
        tracing::debug!(%error, "jwt validation failed");
    })?;

    let email = token_data.claims.email.unwrap_or_default();
    if !emails_match(&email, expected_email) {
        tracing::debug!("jwt email does not match AUTH_USER_EMAIL");
        return Err(());
    }

    let sub = Uuid::parse_str(&token_data.claims.sub).map_err(|error| {
        tracing::debug!(%error, "invalid jwt sub claim");
    })?;

    Ok(AuthUser { sub, email })
}

fn emails_match(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && a.trim().eq_ignore_ascii_case(b.trim())
}
