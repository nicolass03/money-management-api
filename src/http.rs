use std::sync::OnceLock;
use std::time::Duration;

/// Process-wide HTTP client for outbound calls (Supabase JWKS, exchange rates). One client reuses
/// its connection pool, and the timeouts keep a slow upstream from hanging a request until the
/// server's own request timeout.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            // Same failure mode as `reqwest::Client::new()`: only when the TLS backend can't start.
            .expect("HTTP client builds")
    })
}
