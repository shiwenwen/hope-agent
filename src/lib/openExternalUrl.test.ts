import { beforeEach, describe, expect, it, vi } from "vitest"

import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "./openExternalUrl"

vi.mock("@/lib/transport-provider", () => ({
  getTransport: vi.fn(),
}))

const mockedGetTransport = vi.mocked(getTransport)

describe("openExternalUrl", () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it("does not fall back when owner open succeeds", async () => {
    const call = vi.fn().mockResolvedValue(undefined)
    const open = vi.fn()
    mockedGetTransport.mockReturnValue({ call } as never)
    vi.stubGlobal("window", { open })

    openExternalUrl("https://example.test")
    await Promise.resolve()

    expect(call).toHaveBeenCalledWith("open_url", { url: "https://example.test" })
    expect(open).not.toHaveBeenCalled()
  })

  it("reports a final browser-open failure", async () => {
    const call = vi.fn().mockRejectedValue(new Error("owner unavailable"))
    const open = vi.fn().mockReturnValue(null)
    const onError = vi.fn()
    mockedGetTransport.mockReturnValue({ call } as never)
    vi.stubGlobal("window", { open })

    openExternalUrl("https://example.test", { onError })
    await Promise.resolve()
    await Promise.resolve()

    expect(open).toHaveBeenCalledWith("https://example.test", "_blank", "noopener")
    expect(onError).toHaveBeenCalledWith(expect.any(Error))
    expect(String(onError.mock.calls[0]?.[0]?.message)).toContain("Browser blocked")
  })
})
