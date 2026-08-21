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
