use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
const QUOTA_PER_USD: f64 = 500_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BillingMode {
    Tokens,
    Requests,
}

#[derive(Debug, Clone)]
struct SiteModelPrice {
    billing_mode: BillingMode,
    input_price: f64,
    output_price: f64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteModelBillingMode {
    Tokens,
    Requests,
    Mixed,
}

impl Default for SiteModelBillingMode {
    fn default() -> Self {
        Self::Tokens
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SiteModelPriceQuote {
    pub billing_mode: SiteModelBillingMode,
    pub source_count: usize,
    pub from_site_pricing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_per_million: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_per_million: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_per_million_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_per_million_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_price_max: Option<f64>,
}

pub struct SitePriceCache {
    prices: RwLock<HashMap<String, HashMap<String, SiteModelPrice>>>,
    last_refresh: RwLock<HashMap<String, Instant>>,
}

impl SitePriceCache {
    fn new() -> Self {
        Self {
            prices: RwLock::new(HashMap::new()),
            last_refresh: RwLock::new(HashMap::new()),
        }
    }

    pub fn needs_refresh(&self, account_id: &str) -> bool {
        let Ok(guard) = self.last_refresh.read() else {
            return true;
        };
        match guard.get(account_id) {
            Some(last) => last.elapsed() > REFRESH_INTERVAL,
            None => true,
        }
    }

    pub fn set_account_pricing(&self, account_id: &str, payload: &Value) {
        let parsed = parse_pricing_payload(payload);
        if let Ok(mut guard) = self.prices.write() {
            guard.insert(account_id.to_string(), parsed);
        }
        if let Ok(mut guard) = self.last_refresh.write() {
            guard.insert(account_id.to_string(), Instant::now());
        }
    }

    pub fn estimate_cost(
        &self,
        account_id: &str,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> Option<f64> {
        let guard = self.prices.read().ok()?;
        let account_prices = guard.get(account_id)?;

        for candidate in model_lookup_candidates(model) {
            if let Some(price) = account_prices.get(&candidate) {
                let cost = match price.billing_mode {
                    // Request-priced models already use the raw site pricing unit.
                    BillingMode::Requests => price.input_price.max(price.output_price),
                    BillingMode::Tokens => {
                        (price.input_price * input_tokens as f64 + price.output_price * output_tokens as f64)
                            / QUOTA_PER_USD
                    }
                };
                if cost > 0.0 {
                    return Some(cost);
                }
            }
        }

        None
    }

    pub fn quote_model(
        &self,
        account_ids: &[String],
        model: &str,
    ) -> Option<SiteModelPriceQuote> {
        let guard = self.prices.read().ok()?;
        let mut matched_prices = Vec::new();

        if account_ids.is_empty() {
            for account_prices in guard.values() {
                if let Some(price) = lookup_model_price(account_prices, model) {
                    matched_prices.push(price.clone());
                }
            }
        } else {
            for account_id in account_ids {
                let Some(account_prices) = guard.get(account_id) else {
                    continue;
                };
                if let Some(price) = lookup_model_price(account_prices, model) {
                    matched_prices.push(price.clone());
                }
            }
        }

        if matched_prices.is_empty() {
            return None;
        }

        let request_prices: Vec<f64> = matched_prices
            .iter()
            .filter(|price| price.billing_mode == BillingMode::Requests)
            .map(|price| price.input_price.max(price.output_price))
            .filter(|price| *price > 0.0)
            .collect();
        let token_prices: Vec<(f64, f64)> = matched_prices
            .iter()
            .filter(|price| price.billing_mode == BillingMode::Tokens)
            .map(|price| {
                (
                    price.input_price * 1_000_000.0 / QUOTA_PER_USD,
                    price.output_price * 1_000_000.0 / QUOTA_PER_USD,
                )
            })
            .filter(|(input, output)| *input > 0.0 || *output > 0.0)
            .collect();

        let billing_mode = match (!token_prices.is_empty(), !request_prices.is_empty()) {
            (true, false) => SiteModelBillingMode::Tokens,
            (false, true) => SiteModelBillingMode::Requests,
            (true, true) => SiteModelBillingMode::Mixed,
            (false, false) => return None,
        };

        Some(SiteModelPriceQuote {
            billing_mode,
            source_count: matched_prices.len(),
            from_site_pricing: true,
            input_per_million: token_prices.iter().map(|(input, _)| *input).reduce(f64::min),
            output_per_million: token_prices.iter().map(|(_, output)| *output).reduce(f64::min),
            input_per_million_max: token_prices.iter().map(|(input, _)| *input).reduce(f64::max),
            output_per_million_max: token_prices.iter().map(|(_, output)| *output).reduce(f64::max),
            request_price: request_prices.iter().copied().reduce(f64::min),
            request_price_max: request_prices.iter().copied().reduce(f64::max),
        })
    }
}

fn lookup_model_price<'a>(
    account_prices: &'a HashMap<String, SiteModelPrice>,
    model: &str,
) -> Option<&'a SiteModelPrice> {
    for candidate in model_lookup_candidates(model) {
        if let Some(price) = account_prices.get(&candidate) {
            return Some(price);
        }
    }
    None
}

fn parse_pricing_payload(payload: &Value) -> HashMap<String, SiteModelPrice> {
    let data = payload
        .get("data")
        .and_then(|value| value.as_object())
        .or_else(|| payload.as_object());

    let Some(data) = data else {
        return HashMap::new();
    };

    let mut prices = HashMap::new();
    for (model_name, row) in data {
        let quota_type = parse_quota_type(row.get("quota_type"));
        let model_price = row.get("model_price");

        let scalar_price = model_price
            .and_then(value_as_f64)
            .or_else(|| row.get("price").and_then(value_as_f64));

        let nested_input = model_price
            .and_then(|value| value.get("input"))
            .and_then(value_as_f64)
            .or_else(|| row.get("input").and_then(value_as_f64));
        let nested_output = model_price
            .and_then(|value| value.get("output"))
            .and_then(value_as_f64)
            .or_else(|| row.get("output").and_then(value_as_f64));

        let (billing_mode, input_price, output_price) = if quota_type == 1 {
            let request_price = scalar_price.or(nested_input).or(nested_output).unwrap_or(0.0);
            (BillingMode::Requests, request_price, request_price)
        } else {
            let input_price = nested_input.or(scalar_price).unwrap_or(0.0);
            let output_price = nested_output.or(scalar_price).unwrap_or(input_price);
            (BillingMode::Tokens, input_price, output_price)
        };

        if input_price <= 0.0 && output_price <= 0.0 {
            continue;
        }

        prices.insert(
            model_name.trim().to_lowercase(),
            SiteModelPrice {
                billing_mode,
                input_price,
                output_price,
            },
        );
    }

    prices
}

fn parse_quota_type(value: Option<&Value>) -> i64 {
    match value {
        Some(raw) if raw.is_string() => {
            if raw
                .as_str()
                .map(|item| item.eq_ignore_ascii_case("times"))
                .unwrap_or(false)
            {
                1
            } else {
                0
            }
        }
        Some(raw) => raw
            .as_i64()
            .or_else(|| raw.as_u64().map(|item| item as i64))
            .unwrap_or(0),
        None => 0,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|item| item as f64))
        .or_else(|| value.as_u64().map(|item| item as f64))
        .or_else(|| value.as_str().and_then(|item| item.parse::<f64>().ok()))
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

static GLOBAL_SITE_PRICE_CACHE: OnceLock<Arc<SitePriceCache>> = OnceLock::new();

pub fn global() -> Arc<SitePriceCache> {
    GLOBAL_SITE_PRICE_CACHE
        .get_or_init(|| Arc::new(SitePriceCache::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_cost_supports_request_pricing() {
        let cache = SitePriceCache::new();
        cache.set_account_pricing(
            "acc-1",
            &json!({
                "data": {
                    "claude-opus-4.6": {
                        "quota_type": 1,
                        "model_price": 15
                    }
                }
            }),
        );

        let cost = cache.estimate_cost("acc-1", "claude-opus-4.6", 0, 0).unwrap();
        assert!((cost - 15.0).abs() < 1e-12);
    }

    #[test]
    fn estimate_cost_supports_token_pricing() {
        let cache = SitePriceCache::new();
        cache.set_account_pricing(
            "acc-1",
            &json!({
                "data": {
                    "gpt-5.2": {
                        "quota_type": 0,
                        "model_price": {
                            "input": 0.8,
                            "output": 2.0
                        }
                    }
                }
            }),
        );

        let cost = cache.estimate_cost("acc-1", "站点A::gpt-5.2", 1000, 500).unwrap();
        let expected = (0.8 * 1000.0 + 2.0 * 500.0) / QUOTA_PER_USD;
        assert!((cost - expected).abs() < 1e-12);
    }

    #[test]
    fn quote_model_keeps_request_pricing_raw() {
        let cache = SitePriceCache::new();
        cache.set_account_pricing(
            "acc-1",
            &json!({
                "data": {
                    "claude-opus-4.6": {
                        "quota_type": 1,
                        "model_price": 1
                    }
                }
            }),
        );

        let quote = cache
            .quote_model(&["acc-1".to_string()], "claude-opus-4.6")
            .unwrap();

        assert_eq!(quote.billing_mode, SiteModelBillingMode::Requests);
        assert_eq!(quote.request_price, Some(1.0));
        assert_eq!(quote.request_price_max, Some(1.0));
    }
}
