//! Global model price cache fetched from models.dev.
//!
//! Startup: fetch once. Refresh every 24h (checked in auto_cleanup timer).
//! Network failure is completely silent and does not affect proxying.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

const PRICE_API_URL: &str = "https://models.dev/api.json";
const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const COST_PER_MILLION_TOKENS: f64 = 1_000_000.0;

/// Cost per token for a model.
#[derive(Debug, Clone)]
pub struct ModelPrice {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
}

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    models: HashMap<String, ModelsDevModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModelEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    cost: Option<ModelCost>,
}

#[derive(Debug, Deserialize)]
struct ModelCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
}

pub struct PriceCache {
    prices: RwLock<HashMap<String, ModelPrice>>,
    last_refresh: RwLock<Option<Instant>>,
}

impl PriceCache {
    fn new() -> Self {
        Self {
            prices: RwLock::new(HashMap::new()),
            last_refresh: RwLock::new(None),
        }
    }

    /// Check if a refresh is needed (never fetched or >24h old).
    pub fn needs_refresh(&self) -> bool {
        let guard = self.last_refresh.read().unwrap();
        match *guard {
            None => true,
            Some(last) => last.elapsed() > REFRESH_INTERVAL,
        }
    }

    /// Refresh prices from the remote API. Silent on failure.
    pub async fn refresh(&self) {
        tracing::debug!("Refreshing model price cache from {}", PRICE_API_URL);

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(error = %e, "Failed to create HTTP client for price cache");
                return;
            }
        };

        let resp = match client.get(PRICE_API_URL).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::debug!(status = r.status().as_u16(), "Price API returned non-2xx");
                return;
            }
            Err(e) => {
                tracing::debug!(error = %e, "Failed to fetch price data");
                return;
            }
        };

        let body: HashMap<String, ProviderEntry> = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(error = %e, "Failed to parse price data JSON");
                return;
            }
        };

        let prices = extract_prices(&body);
        let count = prices.len();
        if count > 0 {
            if let Ok(mut guard) = self.prices.write() {
                *guard = prices;
            }
            if let Ok(mut guard) = self.last_refresh.write() {
                *guard = Some(Instant::now());
            }
            tracing::info!(model_count = count, "Price cache refreshed");
        }
    }

    /// Estimate cost for a request given model name and token counts.
    /// Returns None if the model is not in the price cache.
    pub fn estimate_cost(
        &self,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> Option<f64> {
        let guard = self.prices.read().ok()?;
        for candidate in model_lookup_candidates(model) {
            if let Some(price) = guard.get(&candidate) {
                let cost = price.input_cost_per_token * input_tokens as f64
                    + price.output_cost_per_token * output_tokens as f64;
                return Some(cost);
            }
        }
        None
    }

    /// Check if the cache has any prices loaded.
    pub fn is_empty(&self) -> bool {
        self.prices.read().map(|g| g.is_empty()).unwrap_or(true)
    }
}

fn extract_prices(body: &HashMap<String, ProviderEntry>) -> HashMap<String, ModelPrice> {
    let mut prices = HashMap::new();

    for provider in body.values() {
        for (model_name, entry) in &provider.models {
            let Some(cost) = &entry.cost else {
                continue;
            };

            let input = cost.input.unwrap_or(0.0) / COST_PER_MILLION_TOKENS;
            let output = cost.output.unwrap_or(0.0) / COST_PER_MILLION_TOKENS;
            if input <= 0.0 && output <= 0.0 {
                continue;
            }

            let normalized_name = entry
                .id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(model_name)
                .to_lowercase();

            prices.entry(normalized_name).or_insert(ModelPrice {
                input_cost_per_token: input,
                output_cost_per_token: output,
            });
        }
    }

    prices
}

fn model_lookup_candidates(model: &str) -> Vec<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    push_candidate(&mut candidates, trimmed);

    if let Some((_, raw_model)) = trimmed.split_once("::") {
        push_candidate(&mut candidates, raw_model);
    }

    if let Some(stripped) = trimmed.strip_prefix("models/") {
        push_candidate(&mut candidates, stripped);
        if let Some((base, _)) = stripped.split_once(':') {
            push_candidate(&mut candidates, base);
        }
    }

    if let Some((base, _)) = trimmed.split_once(':') {
        push_candidate(&mut candidates, base);
    }

    if trimmed.starts_with('[') {
        if let Some(closing) = trimmed.find(']') {
            let remainder = trimmed[closing + 1..].trim();
            if !remainder.is_empty() {
                push_candidate(&mut candidates, remainder);
            }
        }
    }

    candidates
}

fn push_candidate(target: &mut Vec<String>, value: &str) {
    let normalized = value.trim().to_lowercase();
    if !normalized.is_empty() && !target.iter().any(|item| item == &normalized) {
        target.push(normalized);
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static GLOBAL_PRICE_CACHE: OnceLock<Arc<PriceCache>> = OnceLock::new();

pub fn global() -> Arc<PriceCache> {
    GLOBAL_PRICE_CACHE
        .get_or_init(|| Arc::new(PriceCache::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_empty_cache() {
        let cache = PriceCache::new();
        assert!(cache.estimate_cost("gpt-4", 100, 50).is_none());
    }

    #[test]
    fn estimate_cost_with_data() {
        let cache = PriceCache::new();
        {
            let mut guard = cache.prices.write().unwrap();
            guard.insert(
                "gpt-4".to_string(),
                ModelPrice {
                    input_cost_per_token: 0.00003,
                    output_cost_per_token: 0.00006,
                },
            );
        }
        let cost = cache.estimate_cost("gpt-4", 1000, 500).unwrap();
        let expected = 0.00003 * 1000.0 + 0.00006 * 500.0;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn needs_refresh_initially() {
        let cache = PriceCache::new();
        assert!(cache.needs_refresh());
    }

    #[test]
    fn extract_prices_reads_nested_provider_models() {
        let body: HashMap<String, ProviderEntry> = serde_json::from_str(
            r#"{
              "openai": {
                "models": {
                  "gpt-4o-mini": {
                    "id": "gpt-4o-mini",
                    "cost": { "input": 0.15, "output": 0.6 }
                  }
                }
              }
            }"#,
        )
        .unwrap();

        let prices = extract_prices(&body);
        let price = prices.get("gpt-4o-mini").unwrap();
        assert!((price.input_cost_per_token - 0.15 / 1_000_000.0).abs() < 1e-12);
        assert!((price.output_cost_per_token - 0.6 / 1_000_000.0).abs() < 1e-12);
    }

    #[test]
    fn estimate_cost_matches_prefixed_and_gemini_model_variants() {
        let cache = PriceCache::new();
        {
            let mut guard = cache.prices.write().unwrap();
            guard.insert(
                "gpt-5.2".to_string(),
                ModelPrice {
                    input_cost_per_token: 1.0,
                    output_cost_per_token: 2.0,
                },
            );
            guard.insert(
                "gemini-2.5-pro".to_string(),
                ModelPrice {
                    input_cost_per_token: 3.0,
                    output_cost_per_token: 4.0,
                },
            );
        }

        assert_eq!(
            cache.estimate_cost("光速API::gpt-5.2", 2, 3),
            Some(8.0)
        );
        assert_eq!(
            cache.estimate_cost("models/gemini-2.5-pro:generateContent", 1, 1),
            Some(7.0)
        );
    }
}
