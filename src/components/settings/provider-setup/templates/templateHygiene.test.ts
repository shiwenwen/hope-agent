import { describe, expect, it } from "vitest"
import { PROVIDER_TEMPLATES } from "."

const RETIRED_DIRECT_MODEL_IDS = [
  "gpt-5.3-chat-latest",
  "deepseek-chat",
  "deepseek-reasoner",
  "mimo-v2-flash",
  "hy3-preview",
  "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8",
  "moonshotai/Kimi-K2-Instruct-0905",
]

describe("provider template lifecycle hygiene", () => {
  it("applies September 7 retirements only to their public channels", () => {
    const cerebras = PROVIDER_TEMPLATES.find((provider) => provider.key === "cerebras")!
    for (const retired of [
      "gemma-4-31b",
      "zai-glm-4.7",
      "qwen-3-235b-a22b-instruct-2507",
      "llama3.1-8b",
    ]) {
      expect(
        cerebras.models.some((model) => model.id === retired),
        retired,
      ).toBe(false)
    }
    expect(cerebras.models.some((model) => model.id === "gpt-oss-120b")).toBe(true)
    const together = PROVIDER_TEMPLATES.find((provider) => provider.key === "together")!
    expect(together.models.some((model) => model.id === "deepseek-ai/DeepSeek-V4-Pro")).toBe(false)
    for (const retired of ["moonshotai/Kimi-K2.6", "deepseek-ai/DeepSeek-V4-Pro-0813"]) {
      expect(
        together.models.some((model) => model.id === retired),
        retired,
      ).toBe(false)
    }
    expect(
      PROVIDER_TEMPLATES.some(
        (provider) =>
          provider.key !== "together" &&
          provider.models.some((model) => model.id === "deepseek-ai/DeepSeek-V4-Pro"),
      ),
    ).toBe(true)
  })

  it("offers current OpenAI and Anthropic models with their direct API templates", () => {
    const responses = PROVIDER_TEMPLATES.find((provider) => provider.key === "openai")!
    expect(responses.apiType).toBe("openai-responses")
    for (const [id, input, output] of [
      ["gpt-6.1-sol", 2, 10],
      ["gpt-6-sol", 2, 10],
      ["gpt-6-luna", 0.1, 0.5],
    ] as const) {
      expect(responses.models.find((model) => model.id === id)).toMatchObject({
        contextWindow: 1_050_000,
        maxTokens: 128_000,
        inputTypes: ["text", "image"],
        reasoning: true,
        costInput: input,
        costOutput: output,
      })
    }
    expect(responses.models.find((model) => model.id === "gpt-6-astra")).toMatchObject({
      contextWindow: 1_050_000,
      maxTokens: 128_000,
      inputTypes: ["text", "image"],
      reasoning: true,
      costInput: 10,
      costOutput: 50,
    })
    const chat = PROVIDER_TEMPLATES.find((provider) => provider.key === "openai-chat")!
    expect(
      chat.models.some((model) =>
        ["gpt-6.1-sol", "gpt-6-sol", "gpt-6-luna", "gpt-6-astra"].includes(model.id),
      ),
    ).toBe(false)
    const anthropic = PROVIDER_TEMPLATES.find((provider) => provider.key === "anthropic")!
    expect(anthropic.models.find((model) => model.id === "claude-opus-5-5")).toMatchObject({
      contextWindow: 1_000_000,
      maxTokens: 128_000,
      costInput: 4,
      costOutput: 20,
    })
    expect(anthropic.models.find((model) => model.id === "claude-sonnet-5-5")).toMatchObject({
      contextWindow: 1_000_000,
      maxTokens: 128_000,
      costInput: 2,
      costOutput: 10,
    })
    expect(anthropic.models.find((model) => model.id === "claude-fable-5-1")).toMatchObject({
      contextWindow: 1_000_000,
      maxTokens: 128_000,
      costInput: 10,
      costOutput: 50,
    })
    expect(anthropic.models.some((model) => model.id === "claude-mythos-5-1")).toBe(false)
  })
  it("uses the Cloudflare REST endpoint and qualified third-party model IDs", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "cloudflare-ai")
    expect(provider?.baseUrl).toBe(
      "https://api.cloudflare.com/client/v4/accounts/{accountId}/ai/v1",
    )
    expect(provider?.models.length).toBeGreaterThan(0)
    expect(provider?.models.every((model) => model.id.startsWith("anthropic/"))).toBe(true)
  })

  it("removes retired and September 1 Copilot defaults without banning direct models", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "github-copilot")
    expect(provider).toBeDefined()
    for (const id of [
      "claude-opus-4.6",
      "claude-sonnet-4.6",
      "gemini-3.1-pro",
      "gemini-3-flash",
      "gemini-2.5-pro",
      "raptor-mini",
    ]) {
      expect(
        provider?.models.some((model) => model.id === id),
        id,
      ).toBe(false)
    }
    // Account-specific exceptions remain user-configurable; no global ban.
    const anthropic = PROVIDER_TEMPLATES.find((template) => template.key === "anthropic")
    expect(anthropic?.models.some((model) => model.id === "claude-sonnet-4-6")).toBe(true)
  })

  it("keeps the permanent Sonnet 5 base price only in the direct template", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "anthropic")
    expect(provider?.models.find((model) => model.id === "claude-sonnet-5")).toMatchObject({
      costInput: 2,
      costOutput: 10,
    })
  })

  it("uses the canonical multimodal DeepSeek Flash and preserves Pro", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "deepseek")
    expect(provider?.models.find((model) => model.id === "deepseek-flash")).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 384_000,
      reasoning: true,
      costInput: 0.3,
      costOutput: 1.2,
    })
    expect(provider?.models.find((model) => model.id === "deepseek-v4-pro")?.inputTypes).toEqual([
      "text",
    ])
    expect(provider?.models.some((model) => model.id === "deepseek-v4-flash-vision-exp")).toBe(
      false,
    )
  })

  it("removes the exactly identified retired Fireworks serverless presets", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "fireworks")
    expect(
      provider?.models.some((model) => model.id === "accounts/fireworks/models/kimi-k2p6"),
    ).toBe(false)
    for (const retired of [
      "accounts/fireworks/routers/glm-5p2-fast",
      "accounts/fireworks/routers/kimi-k2p6-turbo",
    ]) {
      expect(provider?.models.some((model) => model.id === retired), retired).toBe(false)
    }
    expect(
      provider?.models.some((model) => model.id === "accounts/fireworks/routers/kimi-k2p5-turbo"),
    ).toBe(true)
  })

  it("removes Together's retired Kimi K2.6 serverless preset", () => {
    const together = PROVIDER_TEMPLATES.find((provider) => provider.key === "together")
    expect(together?.models.some((model) => model.id === "moonshotai/Kimi-K2.6")).toBe(false)
  })

  it("does not offer retired direct model IDs to newly configured providers", () => {
    const templateIds = new Set(
      PROVIDER_TEMPLATES.flatMap((provider) => provider.models.map((m) => m.id)),
    )
    for (const retired of RETIRED_DIRECT_MODEL_IDS) {
      expect(templateIds.has(retired), retired).toBe(false)
    }
  })

  it("does not advertise the incomplete Vertex Claude transport", () => {
    expect(PROVIDER_TEMPLATES.some((provider) => provider.key === "anthropic-vertex")).toBe(false)
  })

  it("keeps MiniMax M3 out of the Anthropic-compatible MiniMax template", () => {
    const minimax = PROVIDER_TEMPLATES.find((provider) => provider.key === "minimax")
    expect(minimax).toBeDefined()
    expect(minimax?.models.some((model) => model.id === "MiniMax-M3")).toBe(false)
  })

  it("keeps corrected GPT-5.6 prices in both OpenAI templates", () => {
    for (const key of ["openai", "openai-chat"]) {
      const provider = PROVIDER_TEMPLATES.find((template) => template.key === key)
      for (const id of ["gpt-5.6", "gpt-5.6-sol"]) {
        expect(provider?.models.find((model) => model.id === id)).toMatchObject({
          costInput: 4,
          costOutput: 20,
        })
      }
      expect(provider?.models.find((model) => model.id === "gpt-5.6-terra")).toMatchObject({
        costInput: 2,
        costOutput: 12,
      })
      expect(provider?.models.find((model) => model.id === "gpt-5.6-luna")).toMatchObject({
        costInput: 0.2,
        costOutput: 1.2,
      })
    }
  })

  it("offers Requesty as an OpenAI Chat compatible gateway", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "requesty")
    expect(provider).toMatchObject({
      apiType: "openai-chat",
      baseUrl: "https://router.requesty.ai/v1",
      requiresApiKey: true,
    })
    expect(provider?.models.length).toBeGreaterThan(0)
  })

  it("offers Opper as an OpenAI Chat compatible gateway", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "opper")
    expect(provider).toMatchObject({
      apiType: "openai-chat",
      baseUrl: "https://api.opper.ai/v3/compat/chat/completions",
      requiresApiKey: true,
    })
    expect(provider?.models.length).toBeGreaterThan(0)
    // Pool IDs route across providers and regions, so they carry the most conservative route values.
    expect(provider?.models.find((model) => model.id === "gpt-5.4-mini")).toMatchObject({
      contextWindow: 272_000,
      costInput: 0.825,
      costOutput: 4.95,
    })
  })
})
