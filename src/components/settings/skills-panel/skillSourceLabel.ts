import type { TFunction } from "i18next"

const KNOWN_SKILL_SOURCE_KEYS: Record<string, string> = {
  bundled: "settings.skillSources.bundled",
  shared: "settings.skillSources.shared",
  managed: "settings.skillSources.managed",
  project: "settings.skillSources.project",
}

/** Translate stable built-in source IDs while preserving user directory labels verbatim. */
export function skillSourceLabel(t: TFunction, source: string): string {
  const key = KNOWN_SKILL_SOURCE_KEYS[source]
  return key ? t(key) : source
}
