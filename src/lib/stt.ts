// STT subsystem shared types and transport helpers.

import { getTransport } from "@/lib/transport-provider"

export interface ActiveSttModel {
  providerId: string
  modelId: string
}

/**
 * Mirror of `SttProviderKind` (Rust). Keep in sync with
 * `crates/ha-core/src/stt/types.rs`.
 */
export type SttProviderKind =
  | "openai-transcriptions"
  | "openai-compatible"
  | "openai-chat-completions-asr"
  | "deepgram-ws"
  | "assemblyai-ws"
  | "azure-ws"
  | "volcengine-ws"
  | "xunfei-ws"

/**
 * Mirrors `SttProviderKind::supports_batch()` in ha-core. The desktop
 * voice button's batch path (`stt_transcribe_blob`) and IM auto-transcribe
 * (`failover_transcribe_batch`) can only run on these.
 */
export const BATCH_CAPABLE_KINDS: ReadonlySet<SttProviderKind> = new Set([
  "openai-transcriptions",
  "openai-compatible",
  "openai-chat-completions-asr",
])

/**
 * Realtime WebSocket providers — must be driven via
 * `stt_start_session` / `stt_push_chunk` / `stt_finalize_session` with
 * 16 kHz mono PCM16 chunks. Mirror of every kind NOT in
 * `BATCH_CAPABLE_KINDS`.
 */
export const STREAMING_KINDS: ReadonlySet<SttProviderKind> = new Set([
  "deepgram-ws",
  "assemblyai-ws",
  "azure-ws",
  "volcengine-ws",
  "xunfei-ws",
])

export interface SttProviderSummary {
  id: string
  name: string
  kind: SttProviderKind
  enabled: boolean
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

/**
 * Same Tauri-vs-HTTP shape unwrap for `stt_start_session`. Tauri returns
 * a bare `String` session id; HTTP wraps as `{ sessionId: "..." }`.
 */
export function unwrapSessionId(value: unknown): string {
  if (typeof value === "string") return value
  if (value && typeof value === "object" && "sessionId" in (value as object)) {
    const v = (value as Record<string, unknown>).sessionId
    if (typeof v === "string") return v
  }
  throw new Error("stt_start_session returned unexpected shape")
}

/**
 * Look up the active provider's kind by fetching the provider list and
 * matching by id. Returns `null` when no active model is set or the
 * provider/model is missing (deleted, disabled, etc.). Used by
 * `useVoiceInput` to choose between the batch and streaming code paths.
 */
export async function fetchActiveProviderKind(): Promise<{
  active: ActiveSttModel | null
  kind: SttProviderKind | null
}> {
  const transport = getTransport()
  const [providersRaw, activeRaw] = await Promise.all([
    transport.call<SttProviderSummary[]>("get_stt_providers", {}),
    transport.call<unknown>("get_active_stt_model", {}),
  ])
  const active = unwrapActiveSttModel(activeRaw, "activeModel")
  if (!active) return { active: null, kind: null }
  const provider = (providersRaw ?? []).find((p) => p.id === active.providerId)
  return { active, kind: provider?.kind ?? null }
}
