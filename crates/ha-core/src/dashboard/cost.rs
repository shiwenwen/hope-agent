// ── Cost Estimation ─────────────────────────────────────────────

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
    use super::estimate_cost;

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
