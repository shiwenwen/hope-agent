import { SETTINGS_SECTION_IDS, type SettingsSection } from "./types.ts"

const SETTINGS_SECTION_SET = new Set<string>(SETTINGS_SECTION_IDS)

export function parseOpenSettingsSection(payload: unknown): SettingsSection | undefined {
  if (!payload || typeof payload !== "object") return undefined

  const section = (payload as { section?: unknown }).section
  if (typeof section !== "string") return undefined

  return SETTINGS_SECTION_SET.has(section) ? (section as SettingsSection) : undefined
}
