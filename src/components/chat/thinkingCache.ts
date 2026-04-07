import { getTransport } from "@/lib/transport-provider"

let cachedAutoExpand: boolean | null = null
let cachePromise: Promise<boolean> | null = null

export function getAutoExpandThinking(): Promise<boolean> {
  if (cachedAutoExpand !== null) return Promise.resolve(cachedAutoExpand)
  if (cachePromise) return cachePromise
  cachePromise = getTransport().call<{ autoExpandThinking?: boolean }>("get_user_config")
    .then((cfg) => {
      cachedAutoExpand = cfg.autoExpandThinking !== false
      return cachedAutoExpand
    })
    .catch(() => {
      cachedAutoExpand = true
      return true
    })
  return cachePromise
}

export function getCachedAutoExpandThinking() {
  return cachedAutoExpand
}

export function invalidateThinkingExpandCache() {
  cachedAutoExpand = null
  cachePromise = null
}
