// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { MarkdownLink } from "./MarkdownRenderer"

const mocks = vi.hoisted(() => ({
  transportCall: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: mocks.transportCall }),
}))

vi.mock("@/lib/openExternalUrl", () => ({
  openExternalUrl: vi.fn(),
}))

const observers: MockIntersectionObserver[] = []

class MockIntersectionObserver implements IntersectionObserver {
  readonly callback: IntersectionObserverCallback
  readonly root = null
  readonly rootMargin: string
  readonly scrollMargin = "0px"
  readonly thresholds = [0]

  constructor(callback: IntersectionObserverCallback, options: IntersectionObserverInit = {}) {
    this.callback = callback
    this.rootMargin = options.rootMargin ?? "0px"
    observers.push(this)
  }

  disconnect = vi.fn()
  observe = vi.fn()
  takeRecords = () => []
  unobserve = vi.fn()

  intersect(target: Element) {
    this.callback(
      [{ isIntersecting: true, target } as IntersectionObserverEntry],
      this,
    )
  }
}

beforeEach(() => {
  observers.length = 0
  mocks.transportCall.mockReset().mockResolvedValue({
    dataUrl: "data:image/png;base64,AA==",
  })
  vi.stubGlobal("IntersectionObserver", MockIntersectionObserver)
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("MarkdownLink favicon visibility", () => {
  it("loads a favicon when the link enters the viewport margin", async () => {
    const { container } = render(
      <MarkdownLink href="https://visible-favicon.example/docs">Example</MarkdownLink>,
    )
    const link = container.querySelector("a")

    expect(link).not.toBeNull()
    expect(mocks.transportCall).not.toHaveBeenCalled()
    expect(observers).toHaveLength(1)
    expect(observers[0]?.rootMargin).toBe("160px")

    act(() => observers[0]?.intersect(link!))

    await waitFor(() =>
      expect(mocks.transportCall).toHaveBeenCalledWith("fetch_url_favicon", {
        url: "https://visible-favicon.example/",
      }),
    )
    expect(container.querySelector("img.markdown-link-favicon")).not.toBeNull()
  })

  it("keeps focus as a fallback when IntersectionObserver is unavailable", async () => {
    vi.stubGlobal("IntersectionObserver", undefined)
    const { container } = render(
      <MarkdownLink href="https://focused-favicon.example/docs">Example</MarkdownLink>,
    )
    const link = container.querySelector("a")

    expect(link).not.toBeNull()
    expect(mocks.transportCall).not.toHaveBeenCalled()

    fireEvent.focus(link!)

    await waitFor(() =>
      expect(mocks.transportCall).toHaveBeenCalledWith("fetch_url_favicon", {
        url: "https://focused-favicon.example/",
      }),
    )
  })
})
