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
  it("excludes retired models only from direct OpenAI new-provider presets", () => {
    for (const key of ["openai", "openai-chat"]) {
      const provider = PROVIDER_TEMPLATES.find((template) => template.key === key)!
      for (const id of ["o1", "o1-pro", "o3-mini", "o4-mini", "o3-deep-research", "o4-mini-deep-research"]) {
        expect(provider.models.some((model) => model.id === id), `${key}: ${id}`).toBe(false)
      }
      // Supported alternatives already exist; no automatic saved-model migration.
      for (const id of ["gpt-5.6-sol", "gpt-5.6-terra"]) {
        expect(provider.models.some((model) => model.id === id), `${key}: ${id}`).toBe(true)
      }
    }
  })

  it("offers current stable gateway models with verified channel metadata", () => {
    const together = PROVIDER_TEMPLATES.find((p) => p.key === "together")!
    for (const [id, context, input, output] of [
      ["zai-org/GLM-5.3", 1048575, 1.4, 4.4],
      ["zai-org/GLM-5.3-Flash", 1048575, 0.15, 0.5],
      ["moonshotai/Kimi-K3", 1048576, 2.7, 13.5],
      ["MiniMaxAI/MiniMax-M3", 524288, 0.3, 1.2],
      ["deepseek-ai/DeepSeek-V4.1-Flash", 1000000, 0.3, 1.2],
    ] as const) {
      expect(together.models.find((m) => m.id === id)).toMatchObject({
        inputTypes: ["text"],
        contextWindow: context,
        maxTokens: 16384,
        costInput: input,
        costOutput: output,
        thinkingStyle: "none",
      })
    }
    const router = PROVIDER_TEMPLATES.find((p) => p.key === "openrouter")!
    const ids = [
      "openai/gpt-6-astra",
      "openai/gpt-6.1-sol",
      "openai/gpt-6-sol",
      "openai/gpt-6-luna",
      "anthropic/claude-opus-5.5",
      "anthropic/claude-sonnet-5.5",
      "anthropic/claude-haiku-5.5",
      "x-ai/grok-4.7",
      "z-ai/glm-5.3",
      "z-ai/glm-5.3-flash",
      "deepseek/deepseek-v4.1-flash",
      "minimax/minimax-m3",
      "moonshotai/kimi-k3",
    ]
    for (const id of ids) {
      const model = router.models.find((m) => m.id === id)!
      expect(model, id).toBeDefined()
      expect(model.costInput, id).toBeGreaterThan(0)
      expect(model.contextWindow - model.maxTokens, id).toBeGreaterThan(100_000)
      expect(model.inputTypes, id).not.toContain("video")
      expect(model.inputTypes, id).not.toContain("file")
    }
    expect(router.models.find((m) => m.id === "minimax/minimax-m3")).toMatchObject({
      contextWindow: 524288,
      thinkingStyle: "none",
    })
    // Direct OpenAI tool calls continue to use Responses, rather than a Chat preset.
    expect(
      PROVIDER_TEMPLATES.find((p) => p.key === "openai-chat")?.models.some((m) =>
        m.id.startsWith("gpt-6"),
      ),
    ).toBe(false)
  })

  it("offers GA Z.ai GLM 5.3 Flash through Mistral Chat Completions", () => {
    const mistral = PROVIDER_TEMPLATES.find((provider) => provider.key === "mistral")!
    expect(mistral).toMatchObject({
      apiType: "openai-chat",
      baseUrl: "https://api.mistral.ai/v1",
    })
    expect(mistral.models.find((model) => model.id === "zai-glm-5-3-flash")).toMatchObject({
      name: "Z.ai GLM 5.3 Flash",
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 128_000,
      reasoning: false,
      thinkingStyle: "none",
      costInput: 0.15,
      costOutput: 0.5,
    })
  })

  it("keeps deprecated and existing-user-only Gemini models out of new direct templates", () => {
    const google = PROVIDER_TEMPLATES.find((provider) => provider.key === "google-gemini")!
    for (const id of [
      "gemini-3.7-flash",
      "gemini-3.5-flash",
      "gemini-2.5-pro",
      "gemini-2.5-flash",
      "gemini-2.5-flash-lite",
    ]) {
      expect(
        google.models.some((model) => model.id === id),
        id,
      ).toBe(false)
    }
    expect(google.models.find((model) => model.id === "gemini-4-argon")).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 1_048_576,
      maxTokens: 65_536,
      reasoning: true,
      costInput: 2,
      costOutput: 10,
    })
    expect(google.models.some((model) => model.id === "gemini-3.8-flash")).toBe(true)
    expect(google.models.some((model) => model.id === "gemini-3.6-flash")).toBe(true)
    expect(
      PROVIDER_TEMPLATES.find((provider) => provider.key === "openrouter")?.models.some(
        (model) => model.id === "google/gemini-3.7-flash",
      ),
    ).toBe(true)
  })

  it("offers stable Haiku 5.5 and Grok 4.7 while retaining existing alternatives", () => {
    const anthropic = PROVIDER_TEMPLATES.find((provider) => provider.key === "anthropic")!
    expect(anthropic.apiType).toBe("anthropic")
    expect(anthropic.models.find((model) => model.id === "claude-haiku-5-5")).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 128_000,
      reasoning: true,
      costInput: 0.1,
      costOutput: 0.5,
    })
    expect(anthropic.models.some((model) => model.id === "claude-haiku-4-5")).toBe(true)
    const xai = PROVIDER_TEMPLATES.find((provider) => provider.key === "xai")!
    expect(xai.apiType).toBe("openai-chat")
    expect(xai.models.find((model) => model.id === "grok-4.7")).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 500_000,
      maxTokens: 64_000,
      reasoning: true,
      costInput: 2,
      costOutput: 6,
    })
    for (const retired of [
      "grok-4-1-fast-non-reasoning",
      "grok-4-fast-non-reasoning",
      "grok-4-0709",
      "grok-code-fast-1",
      "grok-3",
    ]) {
      expect(
        xai.models.some((model) => model.id === retired),
        retired,
      ).toBe(false)
    }
    expect(xai.models.some((model) => model.id === "grok-4.3")).toBe(true)
    const grok = xai.models.find((model) => model.id === "grok-4.7")!
    // Output reservation must leave room for admitted prompts and tool history.
    expect(grok.contextWindow - grok.maxTokens).toBeGreaterThan(100_000)
  })

  it("offers verified Fireworks serverless replacements with channel-specific metadata", () => {
    const fireworks = PROVIDER_TEMPLATES.find((provider) => provider.key === "fireworks")!
    expect(fireworks.apiType).toBe("openai-chat")
    for (const [id, modalities, input, output] of [
      ["accounts/fireworks/models/deepseek-v4p1-flash", ["text", "image"], 0.3, 1.2],
      ["accounts/fireworks/models/glm-5p3", ["text"], 1.4, 4.4],
      ["accounts/fireworks/models/glm-5p3-flash", ["text", "image"], 0.15, 0.5],
    ] as const) {
      expect(fireworks.models.find((model) => model.id === id)).toMatchObject({
        contextWindow: 1_048_576,
        maxTokens: 131_072,
        inputTypes: modalities,
        reasoning: true,
        costInput: input,
        costOutput: output,
      })
    }
    // A public serverless model never changes a saved Fire Pass router alias.
    expect(
      fireworks.models.some((model) => model.id === "accounts/fireworks/routers/kimi-k2p5-turbo"),
    ).toBe(true)
  })

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
      expect(
        provider?.models.some((model) => model.id === retired),
        retired,
      ).toBe(false)
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

  it("offers stable MiniMax M3 over the verified compatible API and excludes previews", () => {
    const minimax = PROVIDER_TEMPLATES.find((provider) => provider.key === "minimax")
    expect(minimax).toMatchObject({
      apiType: "anthropic",
      baseUrl: "https://api.minimax.io/anthropic",
    })
    expect(minimax?.models.find((model) => model.id === "MiniMax-M3")).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 131_072,
      reasoning: true,
      costInput: 0.3,
      costOutput: 1.2,
    })
    expect(minimax?.models.some((model) => model.id === "MiniMax-M3.1-Flash-Preview")).toBe(false)
    expect(minimax?.models.some((model) => model.id === "MiniMax-VL-01")).toBe(false)
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
})
