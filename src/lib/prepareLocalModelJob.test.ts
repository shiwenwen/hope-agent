import { beforeEach, describe, expect, it, vi } from "vitest"
import { prepareLocalModelJob } from "./prepareLocalModelJob"

const { call, openExternalUrl } = vi.hoisted(() => ({ call: vi.fn(), openExternalUrl: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => ({ call }) }))
vi.mock("@/lib/openExternalUrl", () => ({ openExternalUrl }))

beforeEach(() => vi.resetAllMocks())

describe("prepareLocalModelJob", () => {
  it.each(["chat_model", "embedding_model", "ollama_install", "ollama_pull", "ollama_preload"] as const)(
    "redirects %s to manual installation without authorizing a job",
    async (kind) => {
      call.mockResolvedValue({ phase: "not-installed", installScriptSupported: false })
      expect(await prepareLocalModelJob(kind)).toBe(false)
      expect(call).toHaveBeenCalledExactlyOnceWith("local_llm_detect_ollama")
      expect(openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://ollama.com/download")
    },
  )

  it("rechecks after the user installs Ollama instead of retaining a stale block", async () => {
    call.mockResolvedValueOnce({ phase: "not-installed", installScriptSupported: false })
      .mockResolvedValueOnce({ phase: "installed", installScriptSupported: false })
    expect(await prepareLocalModelJob("chat_model")).toBe(false)
    expect(await prepareLocalModelJob("chat_model")).toBe(true)
    expect(call).toHaveBeenCalledTimes(2)
    expect(openExternalUrl).toHaveBeenCalledTimes(1)
  })

  it.each(["memory_reembed", "knowledge_reembed"] as const)("does not gate %s on Ollama", async (kind) => {
    expect(await prepareLocalModelJob(kind)).toBe(true)
    expect(call).not.toHaveBeenCalled()
    expect(openExternalUrl).not.toHaveBeenCalled()
  })

  it("propagates detection failure without allowing a job", async () => {
    call.mockRejectedValue(new Error("unavailable"))
    await expect(prepareLocalModelJob("chat_model")).rejects.toThrow("unavailable")
    expect(openExternalUrl).not.toHaveBeenCalled()
  })
})
