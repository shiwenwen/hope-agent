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
  it("uses the Cloudflare REST endpoint and qualified third-party model IDs", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "cloudflare-ai")
    expect(provider?.baseUrl).toBe("https://api.cloudflare.com/client/v4/accounts/{accountId}/ai/v1")
    expect(provider?.models.length).toBeGreaterThan(0)
    expect(provider?.models.every((model) => model.id.startsWith("anthropic/"))).toBe(true)
  })

  it("removes retired and September 1 Copilot defaults without banning direct models", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "github-copilot")
    expect(provider).toBeDefined()
    for (const id of [
      "claude-opus-4.6", "claude-sonnet-4.6", "gemini-3.1-pro",
      "gemini-3-flash", "gemini-2.5-pro", "raptor-mini",
    ]) {
      expect(provider?.models.some((model) => model.id === id), id).toBe(false)
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

  it("advertises only the DeepSeek vision model as image-capable", () => {
    const provider = PROVIDER_TEMPLATES.find((template) => template.key === "deepseek")
    expect(
      provider?.models.find((model) => model.id === "deepseek-v4-flash-vision-exp"),
    ).toMatchObject({
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 384_000,
      reasoning: true,
      costInput: 0.44,
      costOutput: 1.32,
    })
    expect(provider?.models.find((model) => model.id === "deepseek-v4-flash")?.inputTypes).toEqual([
      "text",
    ])
    expect(provider?.models.find((model) => model.id === "deepseek-v4-pro")?.inputTypes).toEqual([
      "text",
    ])
  })

  it("does not offer retired direct model IDs to newly configured providers", () => {
    const templateIds = new Set(PROVIDER_TEMPLATES.flatMap((provider) => provider.models.map((m) => m.id)))
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
})
