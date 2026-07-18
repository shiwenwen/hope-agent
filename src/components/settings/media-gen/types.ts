// TypeScript mirrors of `ha_core::media_gen` (serde camelCase). Keep in sync
// with crates/ha-core/src/media_gen/types.rs.

import type { AvailableModel } from "@/components/ui/model-selector"

export type MediaModality = "image" | "audio" | "video" // "video" reserved, never rendered
export type MediaAudioKind = "speech" | "music" | "sfx"
export type MediaFunctionKey = "image" | MediaAudioKind

export type MediaVendorKind =
  | "openai"
  | "google"
  | "fal"
  | "minimax"
  | "siliconflow"
  | "zhipu"
  | "tongyi"
  | "elevenlabs"
  | "stepfun"
  | "volcengine"
  | "hunyuan"
  | "together"
  | "xai"
  | "recraft"
  | "qianfan"
  | "sensenova"
  | "cartesia"
  | "deepgram"
  | "fishaudio"
  | "hume"
  | "bfl"
  | "stability"
  | "replicate"
  | "kling"
  | "iflytek"
  | "volcengine-tts"
  | "openai-compatible"

export interface ImageEditCaps {
  maxN: number
  maxInputImages: number
  supportsSize: boolean
  supportsAspectRatio: boolean
  supportsResolution: boolean
}

export interface ImageModelCaps {
  maxN: number
  supportsSize: boolean
  supportsAspectRatio: boolean
  supportsResolution: boolean
  sizes: string[]
  aspectRatios: string[]
  resolutions: string[]
  supportsMask: boolean
  edit?: ImageEditCaps | null
}

export interface AudioModelCaps {
  kinds: MediaAudioKind[]
  supportsDuration: boolean
  needsVoice: boolean
  defaultVoice?: string | null
  minDurationSecs?: number | null
  maxDurationSecs?: number | null
}

export interface MediaModelConfig {
  id: string
  name: string
  modality: MediaModality
  image?: ImageModelCaps | null
  audio?: AudioModelCaps | null
  extra: Record<string, string>
}

export interface MediaProviderConfig {
  id: string
  name: string
  kind: MediaVendorKind
  baseUrl?: string | null
  apiKey: string
  enabled: boolean
  models: MediaModelConfig[]
  defaultVoice?: string | null
  allowPrivateNetwork: boolean
  extra: Record<string, string>
}

export interface MediaModelRef {
  providerId: string
  modelId: string
}

export interface MediaModelChain {
  primary: MediaModelRef
  fallbacks: MediaModelRef[]
}

export interface MediaDefaultChains {
  image?: MediaModelChain | null
  speech?: MediaModelChain | null
  music?: MediaModelChain | null
  sfx?: MediaModelChain | null
}

export interface ImageGenDefaults {
  enabled: boolean
  timeoutSeconds: number
  defaultSize: string
  defaultAspectRatio?: string | null
  defaultResolution?: string | null
}

export interface AudioGenDefaults {
  enabled: boolean
  timeoutSeconds: number
  defaultDurationSecs?: number | null
}

export interface MediaGenConfigView {
  providers: MediaProviderConfig[]
  chains: MediaDefaultChains
  imageDefaults: ImageGenDefaults
  audioDefaults: AudioGenDefaults
}

export interface MediaProviderTemplate {
  key: string
  name: string
  kind: MediaVendorKind
  baseUrl: string
  requiresApiKey: boolean
  supportsVoiceListing: boolean
  models: MediaModelConfig[]
}

export interface MediaVoiceOption {
  voiceId: string
  name: string
  category?: string | null
}

export interface MediaCandidateOverview {
  providerId: string
  providerName: string
  vendor: MediaVendorKind
  modelId: string
  modelName: string
  supportsVoiceListing: boolean
  image?: ImageModelCaps | null
  audio?: AudioModelCaps | null
}

export interface MediaFunctionOverview {
  available: boolean
  chainConfigured: boolean
  candidates: MediaCandidateOverview[]
}

export interface MediaGenOverview {
  image: MediaFunctionOverview
  speech: MediaFunctionOverview
  music: MediaFunctionOverview
  sfx: MediaFunctionOverview
  imageDefaults: ImageGenDefaults
  audioDefaults: AudioGenDefaults
}

export const MEDIA_FUNCTION_KEYS: MediaFunctionKey[] = ["image", "speech", "music", "sfx"]

/** Suggested duration buckets (seconds) for music/SFX pickers — mirrors
 *  `media_gen::AUDIO_DURATIONS_SEC`. */
export const AUDIO_DURATION_OPTIONS = [5, 10, 15, 30, 60, 120]

/** Aspect ratios accepted by the request validator — mirrors
 *  `media_gen::VALID_ASPECT_RATIOS`. */
export const VALID_ASPECT_RATIOS = [
  "1:1",
  "2:3",
  "3:2",
  "3:4",
  "4:3",
  "4:5",
  "5:4",
  "9:16",
  "16:9",
  "21:9",
]

export const VALID_RESOLUTIONS = ["1K", "2K", "4K"]

/** Does a model serve a given function key? Lenient like the backend:
 *  audio models without caps (or with empty kinds) serve any audio kind. */
export function modelServes(model: MediaModelConfig, fn: MediaFunctionKey): boolean {
  if (fn === "image") return model.modality === "image"
  if (model.modality !== "audio") return false
  const kinds = model.audio?.kinds ?? []
  return kinds.length === 0 || kinds.includes(fn)
}

/** Usable in candidate lists: enabled + credentials (or a self-hosted
 *  OpenAI-compatible endpoint with a base URL). Mirrors
 *  `MediaProviderConfig::is_usable`. */
export function providerUsable(p: MediaProviderConfig): boolean {
  if (!p.enabled) return false
  if (p.apiKey.trim() !== "") return true
  return p.kind === "openai-compatible" && Boolean(p.baseUrl?.trim())
}

/** Adapt media models to the `AvailableModel` shape `ModelSelector` /
 *  `ModelChainEditor` render. Only providerId/providerName/modelId/modelName
 *  are displayed; the remaining fields are inert placeholders (never pass
 *  `requireVision` alongside this adapter). */
export function toAvailableModels(
  providers: MediaProviderConfig[],
  fn: MediaFunctionKey,
): AvailableModel[] {
  const out: AvailableModel[] = []
  for (const p of providers) {
    if (!providerUsable(p)) continue
    for (const m of p.models) {
      if (!modelServes(m, fn)) continue
      out.push({
        providerId: p.id,
        providerName: p.name,
        apiType: p.kind,
        modelId: m.id,
        modelName: m.name || m.id,
        inputTypes: [],
        contextWindow: 0,
        maxTokens: 0,
        reasoning: false,
      })
    }
  }
  return out
}

/** First auto-mode candidate for a function (provider order × capability
 *  filter, first matching model per provider) — mirrors backend auto
 *  resolution for the "follows provider order" hint. */
export function firstAutoCandidate(
  providers: MediaProviderConfig[],
  fn: MediaFunctionKey,
): { provider: MediaProviderConfig; model: MediaModelConfig } | null {
  for (const p of providers) {
    if (!providerUsable(p)) continue
    const model = p.models.find((m) => modelServes(m, fn))
    if (model) return { provider: p, model }
  }
  return null
}

/** Deep-link to Settings → Model Configuration → Media Generation Models. Mirrors
 *  `openEmbeddingModelSettings()`; requires the App-level `settings:navigate`
 *  listener to forward `modelTab`. */
export function openMediaModelSettings() {
  if (typeof window === "undefined") return
  window.dispatchEvent(
    new CustomEvent("settings:navigate", {
      detail: { section: "modelConfig", modelTab: "mediaModels" },
    }),
  )
}
