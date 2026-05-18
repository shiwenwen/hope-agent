// STT subsystem shared types and transport helpers.

export interface ActiveSttModel {
  providerId: string
  modelId: string
}

/**
 * Tauri returns `Option<T>` (bare T or null); HTTP wraps it in
 * `{ <wrapper>: T | null }`. Normalize both shapes to a flat
 * `ActiveSttModel | null`.
 */
export function unwrapActiveSttModel(
  value: unknown,
  wrapper: "activeModel" | "imFallbackModel",
): ActiveSttModel | null {
  if (value === null || value === undefined) return null
  if (typeof value === "object" && wrapper in (value as object)) {
    const inner = (value as Record<string, unknown>)[wrapper]
    return (inner ?? null) as ActiveSttModel | null
  }
  return value as ActiveSttModel
}
