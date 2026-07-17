// ── Cost Estimation ─────────────────────────────────────────────

/// 结算一次用量的成本。
///
/// 用户可以在设置里逐个模型改单价（`ModelEditor` 的输入/输出成本），所以**用户配置才是
/// 「他实际付多少」的真相源**——此前大盘完全无视配置、只认下面那张内置价目表，用户把价格
/// 改对了大盘照样算错。这里优先按 `(provider_id, model_id)` 回查配置，查不到才回退估算表。
///
/// 按 provider 回查还顺带解决了估算表结构上解不了的问题：同一模型在不同渠道价格不同
/// （kimi-k2.6 直连 $0.95、OpenRouter $0.8），而估算表只按 model_id 匹配、只能存一个值。
///
/// `provider_id` 为 `None` 时（历史行未记录该列）同样回退估算表。
pub(super) fn resolve_cost(
    provider_id: Option<&str>,
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    match provider_id.and_then(|pid| configured_price(pid, model_id)) {
        Some((ci, co)) => (input_tokens as f64 * ci + output_tokens as f64 * co) / 1_000_000.0,
        None => estimate_cost(model_id, input_tokens, output_tokens),
    }
}

/// 从用户配置里取该 provider 下该模型的单价。
///
/// `None` 有两种来源，都回退估算表：provider/模型不在配置里（已删、历史行），或单价为
/// `None`（**未标价**——厂商价未知）。而 `Some(0.0)` 是**明确不按 token 计费**（本地模型、
/// 包月端点），如实结算 $0、**不回退**——旧版把「未标价」和「免费」都写成 `0`，导致本机
/// 跑的 `qwen3:32b` 被按估算表的 $0.30/$0.60 收费。
///
/// 输入/输出任一有值即视为已标价，缺的那侧按 0 计（如只标了输入价）。
fn configured_price(provider_id: &str, model_id: &str) -> Option<(f64, f64)> {
    let cfg = crate::config::cached_config();
    let model = cfg
        .providers
        .iter()
        .find(|p| p.id == provider_id)?
        .models
        .iter()
        .find(|m| m.id == model_id)?;
    match (model.cost_input, model.cost_output) {
        (None, None) => None,
        (ci, co) => Some((ci.unwrap_or(0.0), co.unwrap_or(0.0))),
    }
}

pub(super) fn estimate_cost(model_id: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    // Pricing per 1M tokens: (input_price, output_price)
    let (input_price, output_price) = match model_id {
        // 火山引擎 (豆包) 的 ark id 自带日期后缀，按人民币计价，与同名模型的直连价差几个
        // 数量级。必须排在各厂商臂之前——否则 `glm-4-7-251222` 会被 `glm-4-7` 臂吞掉。
        m if m.contains("doubao-seed-code-preview-251028")
            || m.contains("doubao-seed-1-8-251228")
            || m.contains("kimi-k2-5-260127")
            || m.contains("glm-4-7-251222")
            || m.contains("deepseek-v3-2-251201") =>
        {
            (0.0001, 0.0002)
        }
        // Anthropic — Claude 5 family
        m if m.contains("claude-fable-5") || m.contains("claude-mythos-5") => (10.0, 50.0),
        m if m.contains("claude-sonnet-5") => (3.0, 15.0),
        // Anthropic — Claude 4.x. Opus 4.5 onwards is $5/$25; only Opus 4/4.1 stayed $15/$75.
        m if m.contains("claude-opus-4-8")
            || m.contains("claude-opus-4-7")
            || m.contains("claude-opus-4-6")
            || m.contains("claude-opus-4-5") =>
        {
            (5.0, 25.0)
        }
        m if m.contains("claude-opus-4") => (15.0, 75.0),
        m if m.contains("claude-haiku-4") => (1.0, 5.0),
        m if m.contains("claude-sonnet-4") => (3.0, 15.0),
        // Anthropic — Claude 3.x
        m if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") => (0.80, 4.0),
        m if m.contains("claude-3-opus") || m.contains("claude-3.0-opus") => (15.0, 75.0),
        m if m.contains("claude-3-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-haiku") || m.contains("claude-haiku-3") => (0.25, 1.25),
        m if m.contains("claude-4") => (3.0, 15.0),
        // OpenAI — GPT-5.x. Tier suffixes must precede the bare family arm.
        m if m.contains("gpt-5.6-terra") => (2.5, 15.0),
        m if m.contains("gpt-5.6-luna") => (1.0, 6.0),
        m if m.contains("gpt-5.6") => (5.0, 30.0),
        m if m.contains("gpt-5.5-pro") => (30.0, 180.0),
        m if m.contains("gpt-5.5") => (5.0, 30.0),
        m if m.contains("gpt-5.4-pro") => (30.0, 180.0),
        m if m.contains("gpt-5.4-mini") => (0.75, 4.50),
        m if m.contains("gpt-5.4-nano") => (0.20, 1.25),
        m if m.contains("gpt-5.4") => (2.5, 15.0),
        m if m.contains("gpt-5.3") => (1.75, 14.0),
        // OpenAI
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("gpt-4-turbo") => (10.0, 30.0),
        m if m.contains("gpt-4") => (30.0, 60.0),
        m if m.contains("gpt-3.5") => (0.50, 1.50),
        // OpenAI o-series. `-pro` / `-deep-research` must precede their base arm.
        m if m.contains("o1-pro") => (150.0, 600.0),
        m if m.contains("o1-mini") => (3.0, 12.0),
        m if m.contains("o1") => (15.0, 60.0),
        m if m.contains("o4-mini-deep-research") => (2.0, 8.0),
        m if m.contains("o4-mini") => (1.10, 4.40),
        m if m.contains("o3-mini") => (1.10, 4.40),
        m if m.contains("o3-pro") => (20.0, 80.0),
        m if m.contains("o3-deep-research") => (10.0, 40.0),
        m if m.contains("o3") => (2.0, 8.0),
        // Google Gemini — 3.x. Lite must precede the plain flash arm.
        m if m.contains("gemini-3.1-flash-lite") || m.contains("gemini-3-flash-lite") => {
            (0.10, 0.40)
        }
        m if m.contains("gemini-3.5-flash")
            || m.contains("gemini-3.1-flash")
            || m.contains("gemini-3-flash") =>
        {
            (0.15, 0.60)
        }
        m if m.contains("gemini-3.5-pro")
            || m.contains("gemini-3.1-pro")
            || m.contains("gemini-3-pro") =>
        {
            (1.25, 10.0)
        }
        // Google Gemini. Lite must precede plain flash.
        m if m.contains("gemini-2.5-pro") => (1.25, 10.0),
        m if m.contains("gemini-2.5-flash-lite") => (0.10, 0.40),
        m if m.contains("gemini-2.5-flash") => (0.15, 0.60),
        m if m.contains("gemini-2.0-flash") => (0.10, 0.40),
        m if m.contains("gemini-1.5-pro") => (1.25, 5.0),
        m if m.contains("gemini-1.5-flash") => (0.075, 0.30),
        // xAI Grok. Point releases must precede the `grok-4` / `grok-3` family arms.
        m if m.contains("grok-4.5") => (2.0, 6.0),
        m if m.contains("grok-4.3") => (1.25, 2.5),
        m if m.contains("grok-4.20") => (1.25, 2.5),
        m if m.contains("grok-build") => (1.0, 2.0),
        m if m.contains("grok-4-fast") || m.contains("grok-4-1-fast") => (0.2, 0.5),
        m if m.contains("grok-4") => (3.0, 15.0),
        m if m.contains("grok-3-mini-fast") => (0.6, 4.0),
        m if m.contains("grok-3-mini") => (0.3, 0.5),
        m if m.contains("grok-3-fast") => (5.0, 25.0),
        m if m.contains("grok-3") => (3.0, 15.0),
        m if m.contains("grok-code") => (0.2, 1.5),
        // Mistral
        m if m.contains("codestral") => (0.3, 0.9),
        m if m.contains("devstral") => (0.4, 2.0),
        m if m.contains("magistral") => (0.5, 1.5),
        m if m.contains("pixtral") => (2.0, 6.0),
        m if m.contains("mistral-large") => (0.5, 1.5),
        m if m.contains("mistral-medium-3-5") => (1.5, 7.5),
        m if m.contains("mistral-medium") => (0.4, 2.0),
        m if m.contains("mistral-small") => (0.15, 0.6),
        // DeepSeek. `deepseek-chat` / `-reasoner` now alias the V4 tier.
        m if m.contains("deepseek-v4-pro") || m.contains("DeepSeek-V4-Pro") => (0.435, 0.87),
        m if m.contains("deepseek-v4-flash") || m.contains("DeepSeek-V4-Flash") => (0.14, 0.28),
        m if m.contains("deepseek-chat") || m.contains("deepseek-reasoner") => (0.14, 0.28),
        m if m.contains("DeepSeek-R1") || m.contains("deepseek-r1") => (0.55, 2.19),
        m if m.contains("deepseek") || m.contains("DeepSeek") => (0.27, 1.1),
        // Qwen
        m if m.contains("qwen-max") || m.contains("qwen3-max") => (2.4, 9.6),
        m if m.contains("qwq-plus") => (1.6, 4.0),
        m if m.contains("qwen-plus") => (0.8, 2.0),
        m if m.contains("qwen-turbo") => (0.3, 0.6),
        m if m.contains("qwen") => (0.30, 0.60),
        // GLM (Zhipu)
        m if m.contains("glm-5v-turbo") => (1.2, 4.0),
        m if m.contains("glm-5-turbo") => (1.2, 4.0),
        m if m.contains("glm-5.1") => (1.2, 4.0),
        m if m.contains("glm-5") => (1.0, 3.2),
        m if m.contains("glm-4.7-flashx") => (0.06, 0.4),
        m if m.contains("glm-4.7-flash") => (0.07, 0.4),
        m if m.contains("glm-4.7") || m.contains("glm-4-7") => (0.6, 2.2),
        m if m.contains("glm-4.6v") => (0.3, 0.9),
        m if m.contains("glm-4.6") => (0.6, 2.2),
        m if m.contains("glm-4.5-flash") => (0.0, 0.0),
        m if m.contains("glm-4.5-air") => (0.2, 1.1),
        m if m.contains("glm-4.5v") => (0.6, 1.8),
        m if m.contains("glm-4.5") => (0.6, 2.2),
        // Moonshot Kimi. `kimi-k2-thinking` is billed as K2-era, not K2.5+.
        m if m.contains("kimi-k3") || m.contains("Kimi-K3") => (3.0, 15.0),
        m if m.contains("kimi-k2.7")
            || m.contains("Kimi-K2.7")
            || m.contains("kimi-k2.6")
            || m.contains("Kimi-K2.6")
            || m.contains("kimi-k2p6") =>
        {
            (0.95, 4.0)
        }
        m if m.contains("kimi-k2.5") || m.contains("Kimi-K2.5") || m.contains("kimi-k2p5") => {
            (0.6, 3.0)
        }
        // MiniMax
        m if m.contains("MiniMax-M3") || m.contains("minimax-m3") => (0.6, 2.4),
        m if m.contains("MiniMax-M2.7-highspeed") => (0.6, 2.4),
        m if m.contains("MiniMax") || m.contains("minimax") => (0.3, 1.2),
        // 腾讯混元 (TokenHub)
        m if m.contains("hy3") => (0.176, 0.587),
        // 阶跃星辰 (StepFun). step-3.5-flash 未公布单价，留给默认估价。
        m if m.contains("step-3.7-flash") => (0.2, 1.15),
        // Llama (Together/HuggingFace)
        m if m.contains("Llama-4-Maverick") => (0.27, 0.85),
        m if m.contains("Llama-4-Scout") => (0.18, 0.59),
        m if m.contains("Llama-3.3-70B") || m.contains("llama-3.3-70b") => (0.88, 0.88),
        // Groq
        m if m.contains("mixtral") => (0.24, 0.24),
        _ => (3.0, 15.0), // default estimate
    };
    (input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::{estimate_cost, resolve_cost};
    use crate::config::AppConfig;
    use crate::provider::{ApiType, ModelConfig, ProviderConfig};
    use crate::test_support::replace_config_cache;

    fn model(id: &str, cost_input: Option<f64>, cost_output: Option<f64>) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: id.to_string(),
            input_types: vec!["text".to_string()],
            context_window: 200_000,
            max_tokens: 8_192,
            reasoning: false,
            thinking_style: None,
            cost_input,
            cost_output,
        }
    }

    /// 已标价
    fn priced(id: &str, ci: f64, co: f64) -> ModelConfig {
        model(id, Some(ci), Some(co))
    }

    fn config_with(provider_id: &str, models: Vec<ModelConfig>) -> AppConfig {
        let mut provider = ProviderConfig::new(
            "Test".to_string(),
            ApiType::OpenaiChat,
            "https://example.invalid".to_string(),
            "k".to_string(),
        );
        provider.id = provider_id.to_string();
        provider.models = models;
        AppConfig {
            providers: vec![provider],
            ..Default::default()
        }
    }

    /// 用户在设置里改的单价必须真的影响大盘——这正是本次修复的核心。
    #[test]
    fn configured_price_overrides_the_builtin_table() {
        // 表里 claude-opus-4-8 是 $5/$25；用户配置成 $1/$2。
        let _guard =
            replace_config_cache(config_with("p1", vec![priced("claude-opus-4-8", 1.0, 2.0)]));

        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 1_000_000, 0),
            1.0
        );
        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 0, 1_000_000),
            2.0
        );
        // 未按 provider 解析时仍走内置表，保持既有行为。
        assert_eq!(resolve_cost(None, "claude-opus-4-8", 1_000_000, 0), 5.0);
    }

    /// 同一模型在不同渠道价格不同——这是按 model_id 匹配的估算表结构上解不了的。
    #[test]
    fn same_model_resolves_per_provider() {
        let mut cfg = config_with("direct", vec![priced("kimi-k2.6", 0.95, 4.0)]);
        let mut gateway = ProviderConfig::new(
            "Gateway".to_string(),
            ApiType::OpenaiChat,
            "https://gateway.invalid".to_string(),
            "k".to_string(),
        );
        gateway.id = "gw".to_string();
        gateway.models = vec![priced("kimi-k2.6", 0.8, 3.5)];
        cfg.providers.push(gateway);
        let _guard = replace_config_cache(cfg);

        assert_eq!(
            resolve_cost(Some("direct"), "kimi-k2.6", 1_000_000, 0),
            0.95
        );
        assert_eq!(resolve_cost(Some("gw"), "kimi-k2.6", 1_000_000, 0), 0.80);
    }

    /// 未标价（None）回退估算表——不能把「不知道」静默报成 $0。
    #[test]
    fn unpriced_model_falls_back_to_the_table() {
        let _guard = replace_config_cache(config_with(
            "p1",
            vec![model("claude-opus-4-8", None, None)],
        ));
        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 1_000_000, 0),
            5.0
        );
    }

    /// 明确免费（Some(0.0)）**不回退**，如实记 $0。
    ///
    /// 这条锁住本次修复的核心 bug：本地 Ollama 的 `qwen3:32b` 跑在用户自己机器上、一分钱
    /// 不花，旧版却因为「0 = 未标价」回退到估算表、按 `qwen` 臂的 $0.30/$0.60 收费。
    #[test]
    fn free_model_is_not_billed_at_the_estimated_rate() {
        // 前提：估算表确实会给这些 id 报价——否则本测试无意义。
        assert!(estimate_cost("qwen3:32b", 1_000_000, 0) > 0.0);
        assert!(estimate_cost("your-model-id", 1_000_000, 0) > 0.0);

        let _guard = replace_config_cache(config_with(
            "local",
            vec![
                model("qwen3:32b", Some(0.0), Some(0.0)),
                model("your-model-id", Some(0.0), Some(0.0)),
            ],
        ));

        assert_eq!(
            resolve_cost(Some("local"), "qwen3:32b", 1_000_000, 1_000_000),
            0.0
        );
        assert_eq!(
            resolve_cost(Some("local"), "your-model-id", 1_000_000, 1_000_000),
            0.0
        );
    }

    /// 包月端点（Ollama cloud）与本机推理同属 `Some(0.0)`，代理网关（LiteLLM）属 `None`。
    ///
    /// 这条区分极易改反——`:cloud` 后缀看着像"云端付费"，实则走 ollama.com 包月订阅、
    /// 按 GPU 时长计费，一个 token 单价都不存在；标 `None` 会回退估算表，把 `glm-5.2:cloud`
    /// 按 `glm-5` 臂的直连单价给包月用户报出一笔不存在的账。反过来 LiteLLM 的占位符虽和
    /// vLLM / LM Studio / SGLang 的逐字相同，但它是代理、上游可以是任意付费模型，标 0
    /// 就成了"确定免费"。判据是**计费模式**，不是模型 id 长什么样。
    #[test]
    fn subscription_endpoints_are_free_but_proxy_placeholders_are_unpriced() {
        // 前提：估算表确实会给这两个 id 报价，否则本测试测不出区别。
        assert!(estimate_cost("glm-5.2:cloud", 1_000_000, 0) > 0.0);
        assert!(estimate_cost("your-model-id", 1_000_000, 0) > 0.0);

        let guard = replace_config_cache(config_with(
            "ollama",
            vec![model("glm-5.2:cloud", Some(0.0), Some(0.0))],
        ));
        assert_eq!(
            resolve_cost(Some("ollama"), "glm-5.2:cloud", 1_000_000, 1_000_000),
            0.0,
            "包月端点不按 token 计费，必须如实记 $0 而非回退估算表"
        );
        // guard 持全局互斥锁，shadowing 不提前 drop——不显式释放，下一次 replace 自我死锁。
        drop(guard);

        let _guard = replace_config_cache(config_with(
            "litellm",
            vec![model("your-model-id", None, None)],
        ));
        assert!(
            resolve_cost(Some("litellm"), "your-model-id", 1_000_000, 1_000_000) > 0.0,
            "代理网关单价未知，必须回退估算表而非报 $0"
        );
    }

    /// 只标了一侧价（另一侧留空）仍视为已标价，缺的一侧按 0 计，不整条回退。
    #[test]
    fn half_priced_model_does_not_fall_back() {
        let _guard = replace_config_cache(config_with(
            "p1",
            vec![model("claude-opus-4-8", Some(1.0), None)],
        ));
        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 1_000_000, 0),
            1.0
        );
        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 0, 1_000_000),
            0.0
        );
    }

    /// provider 被删 / 模型已从配置移除 / 历史行无 provider_id —— 都回退，不能算成 0。
    #[test]
    fn unknown_provider_or_model_falls_back_to_the_table() {
        let _guard = replace_config_cache(config_with(
            "p1",
            vec![priced("some-other-model", 1.0, 2.0)],
        ));

        assert_eq!(
            resolve_cost(Some("deleted"), "claude-opus-4-8", 1_000_000, 0),
            5.0
        );
        assert_eq!(
            resolve_cost(Some("p1"), "claude-opus-4-8", 1_000_000, 0),
            5.0
        );
        assert_eq!(resolve_cost(None, "claude-opus-4-8", 1_000_000, 0), 5.0);
    }

    /// Price per 1M tokens, recovered by billing exactly 1M of one kind.
    fn prices(model_id: &str) -> (f64, f64) {
        (
            estimate_cost(model_id, 1_000_000, 0),
            estimate_cost(model_id, 0, 1_000_000),
        )
    }

    /// `estimate_cost` is a first-match-wins substring chain, so a generic arm placed above a
    /// specific one silently swallows it. These cases pin the pairs that actually collide.
    #[test]
    fn specific_arms_win_over_their_generic_family() {
        // `claude-opus-4` must not swallow the 4.5+ models, which repriced to $5/$25.
        assert_eq!(prices("claude-opus-4-8"), (5.0, 25.0));
        assert_eq!(prices("claude-opus-4-7"), (5.0, 25.0));
        assert_eq!(prices("claude-opus-4-6"), (5.0, 25.0));
        assert_eq!(prices("claude-opus-4-5-20251101"), (5.0, 25.0));
        // ...while Opus 4 / 4.1 legitimately stay at the old price.
        assert_eq!(prices("claude-opus-4-1-20250805"), (15.0, 75.0));

        // Tier suffixes differ in price from the bare family.
        assert_eq!(prices("gpt-5.6-terra"), (2.5, 15.0));
        assert_eq!(prices("gpt-5.6-luna"), (1.0, 6.0));
        assert_eq!(prices("gpt-5.6-sol"), (5.0, 30.0));
        assert_eq!(prices("gpt-5.4-mini"), (0.75, 4.50));
        assert_eq!(prices("gpt-5.4-nano"), (0.20, 1.25));
        assert_eq!(prices("gpt-5.5-pro"), (30.0, 180.0));
        assert_eq!(prices("gemini-3.1-flash-lite"), (0.10, 0.40));
        assert_eq!(prices("gemini-2.5-flash-lite"), (0.10, 0.40));
        assert_eq!(prices("o1-pro"), (150.0, 600.0));
        assert_eq!(prices("o3-pro"), (20.0, 80.0));
        assert_eq!(prices("o4-mini-deep-research"), (2.0, 8.0));
        assert_eq!(prices("glm-4.7-flashx"), (0.06, 0.4));
        assert_eq!(prices("glm-4.5-air"), (0.2, 1.1));
        assert_eq!(prices("mistral-medium-3-5"), (1.5, 7.5));
        assert_eq!(prices("qwq-plus"), (1.6, 4.0));

        // `grok-4` must not swallow the point releases, which are priced far below it.
        assert_eq!(prices("grok-4.5"), (2.0, 6.0));
        assert_eq!(prices("grok-4.3"), (1.25, 2.5));
        assert_eq!(prices("grok-4"), (3.0, 15.0));

        // 火山引擎按人民币计价，与同名模型的直连价差几个数量级——绝不能被厂商臂吞掉。
        assert_eq!(prices("glm-4-7-251222"), (0.0001, 0.0002));
        assert_eq!(prices("deepseek-v3-2-251201"), (0.0001, 0.0002));
        assert_eq!(prices("kimi-k2-5-260127"), (0.0001, 0.0002));
        assert_ne!(prices("glm-4-7-251222"), prices("glm-4.7"));
    }

    /// 模板改价后必须同步本表，否则大盘成本与用户实际支出脱节。
    /// 这几项对应 templates/*.ts 里直连厂商的价格，改模板时一并改这里。
    #[test]
    fn estimator_matches_direct_provider_template_prices() {
        assert_eq!(prices("deepseek-reasoner"), (0.14, 0.28));
        assert_eq!(prices("deepseek-chat"), (0.14, 0.28));
        assert_eq!(prices("deepseek-v4-pro"), (0.435, 0.87));
        assert_eq!(prices("deepseek-v4-flash"), (0.14, 0.28));
        assert_eq!(prices("mistral-small-latest"), (0.15, 0.6));
        assert_eq!(prices("mistral-small-2603"), (0.15, 0.6));
        assert_eq!(prices("hy3"), (0.176, 0.587));
        assert_eq!(prices("step-3.7-flash"), (0.2, 1.15));
        assert_eq!(prices("MiniMax-M2.7-highspeed"), (0.6, 2.4));
        assert_eq!(prices("glm-5.1"), (1.2, 4.0));
        assert_eq!(prices("glm-5v-turbo"), (1.2, 4.0));
        assert_eq!(prices("o3"), (2.0, 8.0));
        assert_eq!(prices("grok-build-0.1"), (1.0, 2.0));
        assert_eq!(prices("grok-3-mini-fast"), (0.6, 4.0));
    }

    /// Guards against a current model having no arm at all and silently landing on the fallback.
    /// Only lists models whose real price differs from the default — `claude-sonnet-5` and
    /// `kimi-k3` are genuinely $3/$15, so a match is indistinguishable from a fall-through here;
    /// they are pinned by value in the tests above instead.
    #[test]
    fn current_generation_models_are_not_billed_at_the_default() {
        let default = prices("some-model-nobody-has-priced");
        for id in [
            "claude-fable-5",
            "claude-mythos-5",
            "claude-haiku-4-5-20251001",
            "gpt-5.6",
            "gpt-5.5",
            "gpt-5.4",
            "gemini-3.1-pro-preview",
            "gemini-3.5-flash",
            "kimi-k2.6",
        ] {
            assert_ne!(
                prices(id),
                default,
                "{id} fell through to the default price"
            );
        }
    }

    #[test]
    fn claude_5_family_is_priced_above_opus_tier() {
        assert_eq!(prices("claude-fable-5"), (10.0, 50.0));
        assert_eq!(prices("claude-mythos-5"), (10.0, 50.0));
        assert_eq!(prices("claude-sonnet-5"), (3.0, 15.0));
        assert_eq!(prices("claude-haiku-4-5-20251001"), (1.0, 5.0));
        assert_eq!(prices("claude-sonnet-4-6"), (3.0, 15.0));
    }

    #[test]
    fn cost_scales_with_token_counts() {
        // claude-sonnet-5: $3/1M in, $15/1M out.
        assert!((estimate_cost("claude-sonnet-5", 500_000, 100_000) - (1.5 + 1.5)).abs() < 1e-9);
        assert_eq!(estimate_cost("claude-sonnet-5", 0, 0), 0.0);
    }
}
