// ── Cost Estimation ─────────────────────────────────────────────

pub(super) fn estimate_cost(model_id: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    // Pricing per 1M tokens: (input_price, output_price)
    let (input_price, output_price) = match model_id {
        // Anthropic
        m if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") => (0.80, 4.0),
        m if m.contains("claude-3-opus") || m.contains("claude-3.0-opus") => (15.0, 75.0),
        m if m.contains("claude-3-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-haiku") || m.contains("claude-haiku-3") => (0.25, 1.25),
        m if m.contains("claude-4") || m.contains("claude-sonnet-4") => (3.0, 15.0),
        m if m.contains("claude-opus-4") => (15.0, 75.0),
        // OpenAI
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("gpt-4-turbo") => (10.0, 30.0),
        m if m.contains("gpt-4") => (30.0, 60.0),
        m if m.contains("gpt-3.5") => (0.50, 1.50),
        m if m.contains("o1-mini") => (3.0, 12.0),
        m if m.contains("o1") => (15.0, 60.0),
        m if m.contains("o4-mini") => (1.10, 4.40),
        m if m.contains("o3-mini") => (1.10, 4.40),
        m if m.contains("o3") => (10.0, 40.0),
        // Google Gemini
        m if m.contains("gemini-2.5-pro") => (1.25, 10.0),
        m if m.contains("gemini-2.5-flash") => (0.15, 0.60),
        m if m.contains("gemini-2.0-flash") => (0.10, 0.40),
        m if m.contains("gemini-1.5-pro") => (1.25, 5.0),
        m if m.contains("gemini-1.5-flash") => (0.075, 0.30),
        // xAI Grok
        m if m.contains("grok-4-fast") || m.contains("grok-4-1-fast") => (0.2, 0.5),
        m if m.contains("grok-4.20") => (2.0, 6.0),
        m if m.contains("grok-4") => (3.0, 15.0),
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
        m if m.contains("mistral-medium") => (0.4, 2.0),
        m if m.contains("mistral-small") => (0.1, 0.3),
        // DeepSeek
        m if m.contains("deepseek-reasoner") || m.contains("DeepSeek-R1") => (0.55, 2.19),
        m if m.contains("deepseek") || m.contains("DeepSeek") => (0.27, 1.1),
        // Qwen
        m if m.contains("qwen-max") || m.contains("qwen3-max") => (2.4, 9.6),
        m if m.contains("qwen-plus") || m.contains("qwq-plus") => (0.8, 2.0),
        m if m.contains("qwen-turbo") => (0.3, 0.6),
        m if m.contains("qwen") => (0.30, 0.60),
        // GLM (Zhipu)
        m if m.contains("glm-5-turbo") => (1.2, 4.0),
        m if m.contains("glm-5") => (1.0, 3.2),
        m if m.contains("glm-4.7-flash") => (0.07, 0.4),
        m if m.contains("glm-4.7") || m.contains("glm-4-7") => (0.6, 2.2),
        m if m.contains("glm-4.6v") => (0.3, 0.9),
        m if m.contains("glm-4.6") => (0.6, 2.2),
        m if m.contains("glm-4.5-flash") => (0.0, 0.0),
        m if m.contains("glm-4.5") => (0.6, 2.2),
        // MiniMax
        m if m.contains("MiniMax") || m.contains("minimax") => (0.3, 1.2),
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
