use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
const QUOTA_PER_USD: f64 = 500_000.0;

#[derive(Debug, Clone, Copy)]
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
                let quota = match price.billing_mode {
                    BillingMode::Requests => price.input_price.max(price.output_price),
                    BillingMode::Tokens => {
                        price.input_price * input_tokens as f64 + price.output_price * output_tokens as f64
                    }
                };
                if quota > 0.0 {
                    return Some(quota / QUOTA_PER_USD);
                }
            }
        }

        None
    }
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
        assert!((cost - 15.0 / QUOTA_PER_USD).abs() < 1e-12);
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
}
