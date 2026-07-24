import { getTransport } from "@/lib/transport-provider"

let cachedAutoExpand: boolean | null = null
let cachePromise: Promise<boolean> | null = null

export function getAutoExpandThinking(): Promise<boolean> {
  if (cachedAutoExpand !== null) return Promise.resolve(cachedAutoExpand)
  if (cachePromise) return cachePromise
  cachePromise = getTransport().call<{ autoExpandThinking?: boolean }>("get_user_config")
    .then((cfg) => {
      cachedAutoExpand = cfg.autoExpandThinking === true
      return cachedAutoExpand
    })
    .catch(() => {
      cachedAutoExpand = false
      return false
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
