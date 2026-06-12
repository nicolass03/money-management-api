use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};
use uuid::Uuid;

pub type UserRateLimiter = RateLimiter<Uuid, DashMapStateStore<Uuid>, DefaultClock>;
pub type IpRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

pub fn force_refresh_limiter() -> Arc<UserRateLimiter> {
    let quota = Quota::with_period(Duration::from_secs(60))
        .expect("valid quota")
        .allow_burst(NonZeroU32::new(2).expect("valid burst"));
    Arc::new(RateLimiter::keyed(quota))
}

pub fn auth_failure_limiter() -> Arc<IpRateLimiter> {
    let quota = Quota::with_period(Duration::from_secs(60))
        .expect("valid quota")
        .allow_burst(NonZeroU32::new(10).expect("valid burst"));
    Arc::new(RateLimiter::keyed(quota))
}
