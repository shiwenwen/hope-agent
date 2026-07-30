export const MARKDOWN_FAVICON_ROOT_MARGIN = "160px"

const visibilityCallbacks = new Map<Element, () => void>()
let visibilityObserver: IntersectionObserver | null = null

function disconnectIfIdle(observer: IntersectionObserver) {
  if (visibilityCallbacks.size > 0) return
  observer.disconnect()
  if (visibilityObserver === observer) visibilityObserver = null
}

export function observeMarkdownFaviconVisibility(element: Element, onVisible: () => void) {
  if (typeof IntersectionObserver === "undefined") return () => undefined

  if (!visibilityObserver) {
    visibilityObserver = new IntersectionObserver(
      (entries) => {
        const observer = visibilityObserver
        if (!observer) return
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          const callback = visibilityCallbacks.get(entry.target)
          if (!callback) continue
          visibilityCallbacks.delete(entry.target)
          observer.unobserve(entry.target)
          callback()
        }
        disconnectIfIdle(observer)
      },
      { rootMargin: MARKDOWN_FAVICON_ROOT_MARGIN },
    )
  }

  const observer = visibilityObserver
  visibilityCallbacks.set(element, onVisible)
  observer.observe(element)

  return () => {
    visibilityCallbacks.delete(element)
    observer.unobserve(element)
    disconnectIfIdle(observer)
  }
}
