use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc, Weekday};

/// Model pricing per million tokens in USD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Request properties that materially change a provider's public token price.
/// Provider IDs are OpenCovibe preset IDs, not display names.
#[derive(Clone, Copy, Debug, Default)]
pub struct PricingContext<'a> {
    pub provider_id: Option<&'a str>,
    pub occurred_at: Option<DateTime<FixedOffset>>,
    pub service_tier: Option<&'a str>,
    pub context_tokens: Option<u64>,
}

impl<'a> PricingContext<'a> {
    pub fn for_provider(provider_id: Option<&'a str>) -> Self {
        Self {
            provider_id,
            ..Self::default()
        }
    }

    pub fn at(mut self, occurred_at: DateTime<FixedOffset>) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }
}

/// Model-only lookup retained for historical records that predate provider metadata.
/// Matching is version-bounded: a future model never inherits an older family's price.
pub fn try_get_pricing(model: &str) -> Option<ModelPricing> {
    try_get_pricing_with_context(model, &PricingContext::default())
}

/// Provider-aware pricing lookup. Subscription plans, proxies, local providers, and unknown
/// provider IDs return `None` because their bill cannot be reconstructed from public PAYG rates.
pub fn try_get_pricing_with_context(
    model: &str,
    context: &PricingContext<'_>,
) -> Option<ModelPricing> {
    let normalized = normalize_model(model);

    match context.provider_id {
        Some("anthropic") => claude_model_pricing(&normalized, context),
        Some("openai") => openai_model_pricing(&normalized, context),
        // Native Codex auth does not reveal whether usage is billed as API tokens or ChatGPT
        // credits, so its USD cost cannot be reconstructed from the public API table.
        Some("codex") => None,
        Some("deepseek") => deepseek_pricing(&normalized, context),
        Some("kimi") => kimi_pricing(&normalized),
        Some("minimax" | "minimax-cn") => minimax_pricing(&normalized, context),
        Some("longcat") => longcat_pricing(&normalized),
        Some("zhipu" | "zhipu-intl" | "bailian-api" | "doubao" | "mimo") => None,
        Some(
            "kimi-coding" | "bailian" | "mimo-tp" | "iflytek" | "tencent-coding" | "vercel"
            | "openrouter" | "aihubmix" | "requesty" | "fireworks" | "deepinfra" | "novita"
            | "siliconflow" | "ccswitch" | "ccr" | "ollama",
        ) => None,
        Some(_) => None,
        None => claude_model_pricing(&normalized, context)
            .or_else(|| openai_model_pricing(&normalized, context))
            .or_else(|| deepseek_pricing(&normalized, context))
            .or_else(|| kimi_pricing(&normalized))
            .or_else(|| minimax_pricing(&normalized, context))
            .or_else(|| longcat_pricing(&normalized)),
    }
}

fn normalize_model(model: &str) -> String {
    let lowered = model.trim().to_ascii_lowercase();
    lowered.strip_suffix("[1m]").unwrap_or(&lowered).to_string()
}

/// Match an exact dateless ID or one of its dated snapshots, without matching a future minor
/// version such as `gpt-5.7` when the rule is for `gpt-5`.
fn model_or_snapshot(model: &str, family: &str) -> bool {
    model == family
        || model.strip_prefix(family).is_some_and(|suffix| {
            suffix.len() >= 5
                && suffix.starts_with("-20")
                && suffix[1..].chars().all(|c| c.is_ascii_digit() || c == '-')
        })
}

fn claude_model_pricing(model: &str, context: &PricingContext<'_>) -> Option<ModelPricing> {
    let base = if model_or_snapshot(model, "claude-fable-5")
        || model_or_snapshot(model, "claude-mythos-5")
    {
        claude_pricing(10.0, 50.0)
    } else if [
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
    ]
    .iter()
    .any(|family| model_or_snapshot(model, family))
    {
        claude_pricing(5.0, 25.0)
    } else if ["claude-opus-4-1", "claude-opus-4", "claude-3-opus"]
        .iter()
        .any(|family| model_or_snapshot(model, family))
    {
        claude_pricing(15.0, 75.0)
    } else if model_or_snapshot(model, "claude-sonnet-5") {
        claude_pricing(2.0, 10.0)
    } else if [
        "claude-sonnet-4-6",
        "claude-sonnet-4-5",
        "claude-sonnet-4",
        "claude-3-7-sonnet",
        "claude-3-5-sonnet",
    ]
    .iter()
    .any(|family| model_or_snapshot(model, family))
    {
        claude_pricing(3.0, 15.0)
    } else if model_or_snapshot(model, "claude-haiku-4-5") {
        claude_pricing(1.0, 5.0)
    } else if model_or_snapshot(model, "claude-3-5-haiku") {
        claude_pricing(0.8, 4.0)
    } else {
        return None;
    };

    // Anthropic Priority Tier is committed capacity, not Fast mode. Only the response's explicit
    // `speed` value is passed here by Claude usage readers.
    let fast = context.service_tier == Some("fast");
    if fast
        && ["claude-opus-5", "claude-opus-4-8"]
            .iter()
            .any(|family| model_or_snapshot(model, family))
    {
        return Some(claude_pricing(10.0, 50.0));
    }
    Some(base)
}

fn openai_model_pricing(model: &str, context: &PricingContext<'_>) -> Option<ModelPricing> {
    let long_context = context
        .context_tokens
        .is_some_and(|tokens| tokens > 272_000);
    let mut pricing = if model_or_snapshot(model, "gpt-5.6-sol") || model == "gpt-5.6" {
        if long_context {
            raw_pricing(8.0, 30.0, 0.8, 10.0)
        } else {
            raw_pricing(4.0, 20.0, 0.4, 5.0)
        }
    } else if model_or_snapshot(model, "gpt-5.6-terra") {
        if long_context {
            raw_pricing(4.0, 18.0, 0.4, 5.0)
        } else {
            raw_pricing(2.0, 12.0, 0.2, 2.5)
        }
    } else if model_or_snapshot(model, "gpt-5.6-luna") {
        if long_context {
            raw_pricing(0.4, 1.8, 0.04, 0.5)
        } else {
            raw_pricing(0.2, 1.2, 0.02, 0.25)
        }
    } else if [
        "gpt-5.3-codex",
        "gpt-5.3-chat-latest",
        "gpt-5.2-codex",
        "gpt-5.2",
    ]
    .iter()
    .any(|family| model_or_snapshot(model, family))
    {
        raw_pricing(1.75, 14.0, 0.175, 1.75)
    } else if [
        "gpt-5.1-codex-max",
        "gpt-5.1-codex",
        "gpt-5-codex",
        "gpt-5.1",
        "gpt-5",
    ]
    .iter()
    .any(|family| model_or_snapshot(model, family))
    {
        raw_pricing(1.25, 10.0, 0.125, 1.25)
    } else if ["gpt-5.1-codex-mini", "gpt-5-codex-mini", "gpt-5-mini"]
        .iter()
        .any(|family| model_or_snapshot(model, family))
    {
        raw_pricing(0.25, 2.0, 0.025, 0.25)
    } else if model_or_snapshot(model, "gpt-5.2-pro") {
        raw_pricing(21.0, 168.0, 21.0, 21.0)
    } else if model_or_snapshot(model, "gpt-5.5-pro") {
        long_context_pricing(long_context, 30.0, 180.0, 60.0, 270.0)
    } else if model_or_snapshot(model, "gpt-5.5") {
        long_context_pricing(long_context, 5.0, 30.0, 10.0, 45.0)
    } else if model_or_snapshot(model, "gpt-5.4-pro") {
        long_context_pricing(long_context, 30.0, 180.0, 60.0, 270.0)
    } else if model_or_snapshot(model, "gpt-5.4-mini") {
        raw_pricing(0.75, 4.5, 0.075, 0.75)
    } else if model_or_snapshot(model, "gpt-5.4-nano") {
        raw_pricing(0.2, 1.25, 0.02, 0.2)
    } else if model_or_snapshot(model, "gpt-5.4") {
        long_context_pricing(long_context, 2.5, 15.0, 5.0, 22.5)
    } else if model_or_snapshot(model, "gpt-5-nano") {
        raw_pricing(0.05, 0.4, 0.005, 0.05)
    } else if model_or_snapshot(model, "gpt-5-pro") {
        raw_pricing(15.0, 120.0, 15.0, 15.0)
    } else if model_or_snapshot(model, "gpt-4o-mini") {
        raw_pricing(0.15, 0.6, 0.075, 0.15)
    } else if model == "gpt-4o-2024-05-13" {
        raw_pricing(5.0, 15.0, 5.0, 5.0)
    } else if model_or_snapshot(model, "gpt-4o") {
        raw_pricing(2.5, 10.0, 1.25, 2.5)
    } else if model_or_snapshot(model, "gpt-4.1-mini") {
        raw_pricing(0.4, 1.6, 0.1, 0.4)
    } else if model_or_snapshot(model, "gpt-4.1-nano") {
        raw_pricing(0.1, 0.4, 0.025, 0.1)
    } else if model_or_snapshot(model, "gpt-4.1") {
        raw_pricing(2.0, 8.0, 0.5, 2.0)
    } else if model_or_snapshot(model, "o1-pro") {
        raw_pricing(150.0, 600.0, 150.0, 150.0)
    } else if model_or_snapshot(model, "o1") {
        raw_pricing(15.0, 60.0, 7.5, 15.0)
    } else if model_or_snapshot(model, "o3-pro") {
        raw_pricing(20.0, 80.0, 20.0, 20.0)
    } else if model_or_snapshot(model, "o3-mini") {
        raw_pricing(1.1, 4.4, 0.55, 1.1)
    } else if model_or_snapshot(model, "o3") {
        raw_pricing(2.0, 8.0, 0.5, 2.0)
    } else if model_or_snapshot(model, "o4-mini") {
        raw_pricing(1.1, 4.4, 0.275, 1.1)
    } else {
        return None;
    };

    match context.service_tier {
        Some("fast" | "priority") => {
            let multiplier = if model_or_snapshot(model, "gpt-5.5") {
                if long_context {
                    return None;
                }
                2.5
            } else if [
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.2",
                "gpt-5.1",
                "gpt-5",
            ]
            .iter()
            .any(|family| model_or_snapshot(model, family))
                || model == "gpt-5.6"
            {
                2.0
            } else if model_or_snapshot(model, "gpt-5-mini") {
                pricing = raw_pricing(0.45, 3.6, 0.045, 0.45);
                1.0
            } else if model_or_snapshot(model, "gpt-4.1") {
                pricing = raw_pricing(3.5, 14.0, 0.875, 3.5);
                1.0
            } else if model_or_snapshot(model, "gpt-4.1-mini") {
                pricing = raw_pricing(0.7, 2.8, 0.175, 0.7);
                1.0
            } else if model_or_snapshot(model, "gpt-4.1-nano") {
                pricing = raw_pricing(0.2, 0.8, 0.05, 0.2);
                1.0
            } else if model_or_snapshot(model, "gpt-4o-mini") {
                pricing = raw_pricing(0.25, 1.0, 0.125, 0.25);
                1.0
            } else if model == "gpt-4o-2024-05-13" {
                pricing = raw_pricing(8.75, 26.25, 8.75, 8.75);
                1.0
            } else if model_or_snapshot(model, "gpt-4o") {
                pricing = raw_pricing(4.25, 17.0, 2.125, 4.25);
                1.0
            } else if model_or_snapshot(model, "o3") {
                pricing = raw_pricing(3.5, 14.0, 0.875, 3.5);
                1.0
            } else if model_or_snapshot(model, "o4-mini") {
                pricing = raw_pricing(2.0, 8.0, 0.5, 2.0);
                1.0
            } else {
                return None;
            };
            multiply_pricing(&mut pricing, multiplier);
        }
        Some("flex") => {
            let supports_discount_tier = [
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5-pro",
                "gpt-5.5",
                "gpt-5.4-pro",
                "gpt-5.4-mini",
                "gpt-5.4-nano",
                "gpt-5.4",
                "gpt-5.2",
                "gpt-5.1",
                "gpt-5",
                "gpt-5-mini",
                "gpt-5-nano",
                "o3",
                "o4-mini",
            ]
            .iter()
            .any(|family| model_or_snapshot(model, family));
            if !supports_discount_tier {
                return None;
            }
            if long_context && model_or_snapshot(model, "gpt-5.5-pro") {
                return None;
            }
            multiply_pricing(&mut pricing, 0.5);
        }
        // Interactive OpenCovibe transports do not issue Batch API jobs.
        Some("batch") => return None,
        None | Some("auto" | "default" | "standard") => {}
        // For example, Ultrafast is access-controlled and has no public token price table.
        Some(_) => return None,
    }
    Some(pricing)
}

fn deepseek_pricing(model: &str, context: &PricingContext<'_>) -> Option<ModelPricing> {
    let peak = is_deepseek_peak(
        context
            .occurred_at
            .unwrap_or_else(|| Utc::now().fixed_offset()),
    );
    match model {
        "deepseek-v4-flash" => Some(if peak {
            raw_pricing(0.44, 1.32, 0.014, 0.44)
        } else {
            raw_pricing(0.22, 0.66, 0.007, 0.22)
        }),
        "deepseek-v4-pro" => Some(if peak {
            raw_pricing(1.32, 3.96, 0.044, 1.32)
        } else {
            raw_pricing(0.66, 1.98, 0.022, 0.66)
        }),
        "deepseek-chat" | "deepseek-reasoner" | "deepseek-v3.2" => {
            Some(raw_pricing(0.28, 0.42, 0.028, 0.28))
        }
        _ => None,
    }
}

/// DeepSeek V4 peak time is Beijing 09:00–12:00 and 14:00–18:00 on weekdays.
fn is_deepseek_peak(timestamp: DateTime<FixedOffset>) -> bool {
    let beijing = timestamp.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
    if matches!(beijing.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let minute = beijing.hour() * 60 + beijing.minute();
    (9 * 60..12 * 60).contains(&minute) || (14 * 60..18 * 60).contains(&minute)
}

fn kimi_pricing(model: &str) -> Option<ModelPricing> {
    match model {
        "kimi-k3" => Some(raw_pricing(3.0, 15.0, 0.3, 3.0)),
        "kimi-k2.7-code" => Some(raw_pricing(0.95, 4.0, 0.19, 0.95)),
        "kimi-k2.7-code-highspeed" => Some(raw_pricing(1.9, 8.0, 0.38, 1.9)),
        _ => None,
    }
}

fn minimax_pricing(model: &str, context: &PricingContext<'_>) -> Option<ModelPricing> {
    if model != "minimax-m3" {
        return None;
    }
    let mut pricing = raw_pricing(0.3, 1.2, 0.06, 0.3);
    if context
        .context_tokens
        .is_some_and(|tokens| tokens > 512_000)
    {
        multiply_pricing(&mut pricing, 2.0);
    }
    if matches!(context.service_tier, Some("fast" | "priority")) {
        multiply_pricing(&mut pricing, 1.5);
    }
    Some(pricing)
}

fn longcat_pricing(model: &str) -> Option<ModelPricing> {
    (model == "longcat-2.0").then(|| raw_pricing(0.3, 1.2, 0.006, 0.3))
}

fn claude_pricing(input: f64, output: f64) -> ModelPricing {
    raw_pricing(input, output, input * 0.1, input * 1.25)
}

fn long_context_pricing(
    long: bool,
    short_input: f64,
    short_output: f64,
    long_input: f64,
    long_output: f64,
) -> ModelPricing {
    let (input, output) = if long {
        (long_input, long_output)
    } else {
        (short_input, short_output)
    };
    raw_pricing(input, output, input * 0.1, input)
}

fn raw_pricing(input: f64, output: f64, cache_read: f64, cache_write: f64) -> ModelPricing {
    ModelPricing {
        input,
        output,
        cache_read,
        cache_write,
    }
}

fn multiply_pricing(pricing: &mut ModelPricing, multiplier: f64) {
    pricing.input *= multiplier;
    pricing.output *= multiplier;
    pricing.cache_read *= multiplier;
    pricing.cache_write *= multiplier;
}

/// Known third-party model IDs. This is retained for the Claude protocol boundary, where the
/// upstream CLI reports Anthropic list prices for compatible third-party endpoints.
pub fn is_third_party(model: &str) -> bool {
    let model = normalize_model(model);
    deepseek_pricing(&model, &PricingContext::default()).is_some()
        || kimi_pricing(&model).is_some()
        || minimax_pricing(&model, &PricingContext::default()).is_some()
        || longcat_pricing(&model).is_some()
        || model.starts_with("glm-")
        || model.starts_with("qwen")
        || model.starts_with("doubao-")
        || model.starts_with("mimo-")
}

/// Estimate a known model's cost. Unknown models produce zero only for legacy aggregate callers;
/// new code should use `try_estimate_cost_with_context` to preserve unavailable pricing.
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    try_estimate_cost(
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    )
    .unwrap_or(0.0)
}

pub fn try_estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    try_estimate_cost_with_context(
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        &PricingContext::default(),
    )
}

pub fn try_estimate_cost_with_context(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    context: &PricingContext<'_>,
) -> Option<f64> {
    let pricing = try_get_pricing_with_context(model, context)?;
    Some(compute_cost(
        &pricing,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    ))
}

fn compute_cost(
    pricing: &ModelPricing,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    (input_tokens as f64 * pricing.input
        + output_tokens as f64 * pricing.output
        + cache_read_tokens as f64 * pricing.cache_read
        + cache_write_tokens as f64 * pricing.cache_write)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }

    #[test]
    fn future_versions_do_not_inherit_family_prices() {
        assert!(try_get_pricing("claude-opus-5").is_some());
        assert!(try_get_pricing("claude-opus-5-20260724").is_some());
        assert!(try_get_pricing("claude-opus-6").is_none());
        assert!(try_get_pricing("gpt-5.6-sol").is_some());
        assert!(try_get_pricing("gpt-5.7-sol").is_none());
        assert!(try_get_pricing("deepseek-v5-pro").is_none());
    }

    #[test]
    fn provider_context_blocks_proxy_and_subscription_payg_prices() {
        let proxy = PricingContext::for_provider(Some("openrouter"));
        assert!(try_get_pricing_with_context("claude-opus-5", &proxy).is_none());
        let plan = PricingContext::for_provider(Some("bailian"));
        assert!(try_get_pricing_with_context("qwen3.7-plus", &plan).is_none());
        let native_codex = PricingContext::for_provider(Some("codex"));
        assert!(try_get_pricing_with_context("gpt-5.6-sol", &native_codex).is_none());
    }

    #[test]
    fn claude_and_gpt_56_prices_are_version_specific() {
        assert_eq!(
            try_get_pricing("claude-sonnet-5"),
            Some(claude_pricing(2.0, 10.0))
        );
        assert_eq!(
            try_get_pricing("claude-haiku-4-5-20251001"),
            Some(claude_pricing(1.0, 5.0))
        );
        assert_eq!(
            try_get_pricing("gpt-5.6-terra"),
            Some(raw_pricing(2.0, 12.0, 0.2, 2.5))
        );
    }

    #[test]
    fn gpt_56_long_context_and_fast_tiers_stack() {
        let context = PricingContext {
            provider_id: Some("openai"),
            service_tier: Some("fast"),
            context_tokens: Some(300_000),
            ..PricingContext::default()
        };
        assert_eq!(
            try_get_pricing_with_context("gpt-5.6-luna", &context),
            Some(raw_pricing(0.8, 3.6, 0.08, 1.0))
        );
    }

    #[test]
    fn deepseek_v4_uses_beijing_peak_boundaries() {
        let off_peak =
            PricingContext::for_provider(Some("deepseek")).at(at("2026-08-25T08:59:59+08:00"));
        let peak =
            PricingContext::for_provider(Some("deepseek")).at(at("2026-08-25T09:00:00+08:00"));
        let noon =
            PricingContext::for_provider(Some("deepseek")).at(at("2026-08-25T12:00:00+08:00"));

        assert_eq!(
            try_get_pricing_with_context("deepseek-v4-flash", &off_peak),
            Some(raw_pricing(0.22, 0.66, 0.007, 0.22))
        );
        assert_eq!(
            try_get_pricing_with_context("deepseek-v4-flash", &peak),
            Some(raw_pricing(0.44, 1.32, 0.014, 0.44))
        );
        assert_eq!(
            try_get_pricing_with_context("deepseek-v4-flash", &noon),
            Some(raw_pricing(0.22, 0.66, 0.007, 0.22))
        );
    }

    #[test]
    fn deepseek_weekends_are_always_off_peak() {
        let saturday =
            PricingContext::for_provider(Some("deepseek")).at(at("2026-08-22T10:00:00+08:00"));
        let sunday =
            PricingContext::for_provider(Some("deepseek")).at(at("2026-08-23T10:00:00+08:00"));
        assert_eq!(
            try_get_pricing_with_context("deepseek-v4-pro", &saturday)
                .unwrap()
                .input,
            0.66
        );
        assert_eq!(
            try_get_pricing_with_context("deepseek-v4-pro", &sunday)
                .unwrap()
                .input,
            0.66
        );
    }

    #[test]
    fn openai_fast_prices_are_model_specific() {
        let fast = PricingContext {
            provider_id: Some("openai"),
            service_tier: Some("fast"),
            ..PricingContext::default()
        };
        assert_eq!(
            try_get_pricing_with_context("gpt-5.5", &fast),
            Some(raw_pricing(12.5, 75.0, 1.25, 12.5))
        );
        assert_eq!(
            try_get_pricing_with_context("gpt-5-mini", &fast),
            Some(raw_pricing(0.45, 3.6, 0.045, 0.45))
        );
        assert_eq!(
            try_get_pricing_with_context("gpt-4o-2024-05-13", &fast),
            Some(raw_pricing(8.75, 26.25, 8.75, 8.75))
        );
        assert!(try_get_pricing_with_context("gpt-5.5-pro", &fast).is_none());
        assert!(try_get_pricing_with_context("gpt-5-codex-mini", &fast).is_none());

        let long_fast = PricingContext {
            context_tokens: Some(300_000),
            ..fast
        };
        assert!(try_get_pricing_with_context("gpt-5.5", &long_fast).is_none());
    }

    #[test]
    fn claude_priority_tier_is_not_fast_mode() {
        let priority = PricingContext {
            provider_id: Some("anthropic"),
            service_tier: Some("priority"),
            ..PricingContext::default()
        };
        assert_eq!(
            try_get_pricing_with_context("claude-opus-4-8", &priority),
            Some(claude_pricing(5.0, 25.0))
        );
    }

    #[test]
    fn openai_flex_and_batch_support_are_not_conflated() {
        let flex = PricingContext {
            provider_id: Some("openai"),
            service_tier: Some("flex"),
            ..PricingContext::default()
        };
        assert_eq!(
            try_get_pricing_with_context("gpt-5.6-sol", &flex),
            Some(raw_pricing(2.0, 10.0, 0.2, 2.5))
        );
        assert!(try_get_pricing_with_context("gpt-5.3-codex", &flex).is_none());

        let batch = PricingContext {
            service_tier: Some("batch"),
            ..flex
        };
        assert!(try_get_pricing_with_context("gpt-5.6-sol", &batch).is_none());

        let ultrafast = PricingContext {
            service_tier: Some("ultrafast"),
            ..flex
        };
        assert!(try_get_pricing_with_context("gpt-5.6-sol", &ultrafast).is_none());
    }
}
