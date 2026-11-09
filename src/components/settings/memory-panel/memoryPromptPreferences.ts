export type MemoryPromptStyle = "default" | "concise" | "detailed"
export interface MemoryPromptPreference {
  style: MemoryPromptStyle
  supplemental: string
}
export type MemoryPromptStage = "extraction" | "profile" | "dreaming"
export type MemoryPromptPreferences = Record<MemoryPromptStage, MemoryPromptPreference>
export type MemoryPromptOverrides = Partial<Record<MemoryPromptStage, MemoryPromptPreference | null>>

export const DEFAULT_MEMORY_PROMPT_PREFERENCES: MemoryPromptPreferences = {
  extraction: { style: "default", supplemental: "" },
  profile: { style: "default", supplemental: "" },
  dreaming: { style: "default", supplemental: "" },
}
