import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import {
  MARKDOWN_FAVICON_ROOT_MARGIN,
  observeMarkdownFaviconVisibility,
} from "./markdownFaviconVisibility"

const observers: MockIntersectionObserver[] = []

class MockIntersectionObserver {
  readonly callback: IntersectionObserverCallback
  readonly options: IntersectionObserverInit
  readonly disconnect = vi.fn()
  readonly observe = vi.fn()
  readonly unobserve = vi.fn()

  constructor(callback: IntersectionObserverCallback, options: IntersectionObserverInit) {
    this.callback = callback
    this.options = options
    observers.push(this)
  }

  intersect(target: Element, isIntersecting: boolean) {
    this.callback(
      [{ isIntersecting, target } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }
}

beforeEach(() => {
  observers.length = 0
  vi.stubGlobal("IntersectionObserver", MockIntersectionObserver)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("observeMarkdownFaviconVisibility", () => {
  it("activates only after the link enters the viewport margin", () => {
    const element = {} as Element
    const onVisible = vi.fn()

    const cleanup = observeMarkdownFaviconVisibility(element, onVisible)
    const observer = observers[0]!

    expect(observer.options.rootMargin).toBe(MARKDOWN_FAVICON_ROOT_MARGIN)
    expect(observer.observe).toHaveBeenCalledWith(element)

    observer.intersect(element, false)
    expect(onVisible).not.toHaveBeenCalled()

    observer.intersect(element, true)
    expect(onVisible).toHaveBeenCalledOnce()
    expect(observer.unobserve).toHaveBeenCalledWith(element)
    expect(observer.disconnect).toHaveBeenCalledOnce()

    cleanup()
  })

  it("leaves interaction handlers as the fallback without IntersectionObserver", () => {
    vi.stubGlobal("IntersectionObserver", undefined)
    const onVisible = vi.fn()

    const cleanup = observeMarkdownFaviconVisibility({} as Element, onVisible)

    expect(onVisible).not.toHaveBeenCalled()
    expect(cleanup()).toBeUndefined()
  })
})
