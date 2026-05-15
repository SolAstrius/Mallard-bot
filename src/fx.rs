//! Exchange-rate cache and provider for `/calc`.
//!
//! Fend's exchange-rate hook is synchronous and needs a (currency_code →
//! rate-vs-base) lookup on demand. We back it with a process-wide
//! `OnceLock<RateProvider>` whose internal table is refreshed by a Tokio
//! background task every [`REFRESH_EVERY`].
//!
//! Base currency: **USD**. Open rates come from `open.er-api.com`, which
//! is auth-free and updated daily. If the upstream is unreachable the
//! cache simply doesn't get updated; the handler returns the last good
//! table until the next refresh succeeds.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use serde::Deserialize;

const BASE: &str = "USD";
const SOURCE_URL: &str = "https://open.er-api.com/v6/latest/USD";
const REFRESH_EVERY: Duration = Duration::from_secs(6 * 60 * 60); // 6h
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct RateTable {
    /// Maps currency code (uppercased, e.g. "EUR") to rate relative to
    /// the base currency (USD = 1.0).
    pub rates: HashMap<String, f64>,
    /// Unix timestamp when the upstream's data was last updated.
    pub upstream_unix: i64,
    /// Unix timestamp when *we* fetched it.
    pub fetched_unix: i64,
}

#[derive(Debug, Deserialize)]
struct OpenErApiResponse {
    result: String,
    base_code: String,
    time_last_update_unix: i64,
    rates: HashMap<String, f64>,
}

#[derive(Default)]
pub struct RateProvider {
    inner: RwLock<Option<RateTable>>,
}

impl RateProvider {
    pub fn get_rate(&self, currency: &str) -> Option<f64> {
        let upper = currency.to_ascii_uppercase();
        let guard = self.inner.read().ok()?;
        let table = guard.as_ref()?;
        if upper == BASE {
            return Some(1.0);
        }
        table.rates.get(&upper).copied()
    }

    pub fn last_fetch(&self) -> Option<RateTable> {
        self.inner.read().ok().and_then(|g| g.clone())
    }
}

static PROVIDER: OnceLock<Arc<RateProvider>> = OnceLock::new();

pub fn provider() -> Arc<RateProvider> {
    PROVIDER
        .get_or_init(|| Arc::new(RateProvider::default()))
        .clone()
}

/// Spawn the background refresher. Idempotent — calling more than once
/// just chains another task, but typically we call once from `main`.
pub fn spawn_refresher() {
    let prov = provider();
    tokio::spawn(async move {
        // Eager first fetch; failures here are not fatal.
        if let Err(e) = refresh_once(&prov).await {
            log::warn!("fx initial fetch failed: {e}");
        }
        loop {
            tokio::time::sleep(REFRESH_EVERY).await;
            if let Err(e) = refresh_once(&prov).await {
                log::warn!("fx refresh failed: {e}");
            }
        }
    });
}

async fn refresh_once(prov: &RateProvider) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("mallard-bot")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(SOURCE_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("upstream HTTP {}", resp.status()));
    }
    let body: OpenErApiResponse = resp.json().await.map_err(|e| e.to_string())?;
    if body.result != "success" || body.base_code != BASE {
        return Err(format!(
            "unexpected payload: result={} base={}",
            body.result, body.base_code
        ));
    }
    let table = RateTable {
        rates: body.rates,
        upstream_unix: body.time_last_update_unix,
        fetched_unix: chrono::Utc::now().timestamp(),
    };
    log::info!(
        "fx refreshed: {} currencies (upstream ts {})",
        table.rates.len(),
        table.upstream_unix
    );
    if let Ok(mut w) = prov.inner.write() {
        *w = Some(table);
    }
    Ok(())
}

/// Fend trait impl: takes a currency code, returns its value relative to
/// the base (USD). Fend does the cross-currency math itself; we just
/// expose a one-dimensional lookup.
pub struct FendRateHandler {
    pub provider: Arc<RateProvider>,
}

impl fend_core::ExchangeRateFnV2 for FendRateHandler {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // Fend asks "how much of the base currency is one unit of <currency>".
        // open.er-api gives the inverse: how much <currency> per 1 USD.
        // So return reciprocal.
        match self.provider.get_rate(currency) {
            Some(r) if r > 0.0 => Ok(1.0 / r),
            Some(_) => Err("rate is zero".into()),
            None => Err(format!("неизвестная валюта: {currency}").into()),
        }
    }
}
