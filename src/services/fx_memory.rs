use std::sync::{OnceLock, RwLock};

use crate::services::currency::ExchangeRates;
use crate::services::exchange_rates::is_rates_stale;

static FX_MEMORY: OnceLock<RwLock<Option<ExchangeRates>>> = OnceLock::new();

fn fx_memory() -> &'static RwLock<Option<ExchangeRates>> {
    FX_MEMORY.get_or_init(|| RwLock::new(None))
}

pub fn get_memory_rates(force_refresh: bool) -> Option<ExchangeRates> {
    if force_refresh {
        return None;
    }
    let guard = fx_memory().read().ok()?;
    guard
        .as_ref()
        .filter(|rates| !is_rates_stale(&rates.fetched_at))
        .cloned()
}

pub fn set_memory_rates(rates: ExchangeRates) {
    if let Ok(mut guard) = fx_memory().write() {
        *guard = Some(rates);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cache_round_trip() {
        let rates = ExchangeRates {
            base: "USD".into(),
            rates: Default::default(),
            fetched_at: chrono::Utc::now().to_rfc3339(),
        };
        set_memory_rates(rates.clone());
        let cached = get_memory_rates(false).expect("cached rates");
        assert_eq!(cached.base, rates.base);
    }
}
