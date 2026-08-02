// @vitest-environment jsdom

import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { ArtifactThumb } from "./ArtifactThumb"

const transportState = vi.hoisted(() => {
  const transport = {
    call: vi.fn(),
    listen: vi.fn(() => () => undefined),
    resolveAssetUrl: vi.fn(),
  }
  return { revision: 0, ticket: "old-ticket", transport }
})

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportState.transport,
  getTransportRevision: () => transportState.revision,
  useTransportRevision: () => transportState.revision,
}))

vi.mock("@/lib/designThumbPool", () => ({
  acquireThumb: vi.fn(),
  setThumbVisible: vi.fn(),
  releaseThumb: vi.fn(),
}))

class VisibleIntersectionObserver {
  readonly disconnect = vi.fn()
  private readonly callback: IntersectionObserverCallback

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback
  }

  observe(target: Element) {
    this.callback(
      [{ isIntersecting: true, target } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }
}

class PassiveResizeObserver {
  readonly disconnect = vi.fn()
  observe() {
    return undefined
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal("IntersectionObserver", VisibleIntersectionObserver)
  vi.stubGlobal("ResizeObserver", PassiveResizeObserver)
  transportState.revision = 0
  transportState.ticket = "old-ticket"
  transportState.transport.call.mockResolvedValue({
    artifactPath: "/design/artifact",
    currentVersion: 7,
  })
  transportState.transport.resolveAssetUrl.mockImplementation(
    (path: string) => `https://agent.example/api/resource/${transportState.ticket}${path}`,
  )
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

describe("ArtifactThumb", () => {
  it("does not reuse a cached scoped URL after the transport ticket changes", async () => {
    const first = render(<ArtifactThumb artifactId="artifact-ticket-refresh" />)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(350)
    })
    expect(first.container.querySelector("iframe")?.getAttribute("src")).toContain("old-ticket")
    first.unmount()

    transportState.revision += 1
    transportState.ticket = "new-ticket"
    const second = render(<ArtifactThumb artifactId="artifact-ticket-refresh" />)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(350)
    })

    const src = second.container.querySelector("iframe")?.getAttribute("src")
    expect(src).toContain("new-ticket")
    expect(src).not.toContain("old-ticket")
    expect(transportState.transport.call).toHaveBeenCalledTimes(2)
  })
})
