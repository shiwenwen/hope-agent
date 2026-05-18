import type { SttProviderKind } from "@/lib/stt"

/**
 * STT provider preset registry — the source of truth for the API-type
 * dropdown in [`VoicePanel`](./VoicePanel.tsx).
 *
 * Multiple presets can share a wire-level `kind`. 阿里云百炼
 * (DashScope) and the generic Chat Completions ASR both dispatch
 * through `openai-chat-completions-asr` on the backend; they differ
 * only in defaults the user would otherwise have to fill by hand.
 */
export interface SttKindPreset {
  /** Unique dropdown value. May differ from `kind` (e.g. DashScope is
   * a UI preset of `openai-chat-completions-asr`). */
  slug: string
  /** Underlying wire protocol — what the backend dispatches on. */
  kind: SttProviderKind
  /** Transport layer — drives Base URL scheme validation. */
  transport: "ws" | "http"
  /** English brand / canonical name shown in the dropdown. */
  brand: string
  /** Optional Chinese / localised name shown before the brand
   * (`{chinese} · {brand}` layout). Omit for already-localised brands. */
  chineseName?: string
  /** Short protocol tag shown in parens, e.g. "WS" / "Chat Completions ASR". */
  protocol?: string
  /** Provider icon key looked up by `<ProviderIcon>`. Falls back to a
   * generic settings glyph when undefined. */
  iconKey?: string
  /** Pre-filled when the user picks this preset on a fresh provider. */
  defaultBaseUrl: string
  /** Whether Base URL is mandatory at save time. Cloud providers with
   * a shipped default ("openai-transcriptions" / WS hosts) don't need
   * the user to type one; tenant-specific kinds do. */
  requiresBaseUrl: boolean
  /** Pre-filled model list. Activation flow needs at least one row;
   * kinds whose wire has no real "model id" (iFlytek `domain`,
   * Volcengine `bigmodel`) seed meaningful presets. */
  defaultModels: Array<{ id: string; name: string }>
}

export const STT_PRESETS: SttKindPreset[] = [
  {
    slug: "openai-transcriptions",
    kind: "openai-transcriptions",
    transport: "http",
    brand: "OpenAI Audio Transcriptions",
    iconKey: "openai",
    defaultBaseUrl: "https://api.openai.com",
    requiresBaseUrl: false,
    // gpt-4o-(mini-)transcribe are the latest GA models per
    // platform.openai.com/docs/api-reference/audio; whisper-1 is the
    // legacy fallback for older accounts.
    defaultModels: [
      { id: "gpt-4o-transcribe", name: "GPT-4o Transcribe" },
      { id: "gpt-4o-mini-transcribe", name: "GPT-4o mini Transcribe" },
      { id: "whisper-1", name: "Whisper-1 (legacy)" },
    ],
  },
  {
    slug: "openai-compatible",
    kind: "openai-compatible",
    transport: "http",
    brand: "OpenAI-compatible",
    iconKey: "openai",
    defaultBaseUrl: "",
    requiresBaseUrl: true,
    // Empty — third-party compatible servers (Groq, whisper.cpp,
    // faster-whisper-server, FunASR wrappers) use different model
    // names; pre-seeding would mislead.
    defaultModels: [],
  },
  {
    slug: "openai-chat-completions-asr",
    kind: "openai-chat-completions-asr",
    transport: "http",
    brand: "Chat Completions ASR (input_audio)",
    iconKey: "openai",
    defaultBaseUrl: "",
    requiresBaseUrl: true,
    defaultModels: [],
  },
  {
    slug: "dashscope",
    kind: "openai-chat-completions-asr",
    transport: "http",
    brand: "DashScope / Bailian",
    chineseName: "阿里云百炼",
    protocol: "Chat Completions ASR",
    iconKey: "dashscope",
    defaultBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode",
    requiresBaseUrl: false,
    defaultModels: [{ id: "qwen3-asr-flash", name: "Qwen3-ASR Flash" }],
  },
  {
    slug: "deepgram-ws",
    kind: "deepgram-ws",
    transport: "ws",
    brand: "Deepgram",
    protocol: "WS",
    iconKey: "deepgram",
    defaultBaseUrl: "wss://api.deepgram.com",
    requiresBaseUrl: false,
    // Per `ListenV1Model` enum at developers.deepgram.com — nova-3 is
    // the latest GA, nova-2 family stays for domain-specialised variants.
    defaultModels: [
      { id: "nova-3", name: "Nova-3 (latest)" },
      { id: "nova-3-medical", name: "Nova-3 Medical" },
      { id: "nova-2", name: "Nova-2" },
      { id: "nova-2-meeting", name: "Nova-2 Meeting" },
      { id: "nova-2-phonecall", name: "Nova-2 Phone Call" },
    ],
  },
  {
    slug: "assemblyai-ws",
    kind: "assemblyai-ws",
    transport: "ws",
    brand: "AssemblyAI",
    protocol: "WS",
    iconKey: "assemblyai",
    defaultBaseUrl: "wss://streaming.assemblyai.com",
    requiresBaseUrl: false,
    // `speech_model` enum from the v3 Universal Streaming AsyncAPI spec.
    defaultModels: [
      { id: "universal-streaming-english", name: "Universal Streaming · English" },
      {
        id: "universal-streaming-multilingual",
        name: "Universal Streaming · Multilingual",
      },
      { id: "whisper-rt", name: "Whisper-RT (99+ languages)" },
    ],
  },
  {
    slug: "azure-ws",
    kind: "azure-ws",
    transport: "ws",
    brand: "Azure Speech",
    chineseName: "微软语音",
    protocol: "WS",
    iconKey: "azure",
    defaultBaseUrl: "",
    // Region is set via `extra.region`, which synthesises the base
    // URL — so users don't have to paste anything explicit.
    requiresBaseUrl: false,
    // Azure has no wire-level model id (region + language drives it).
    // Single sentinel row keeps the activation flow happy.
    defaultModels: [{ id: "default", name: "默认识别引擎 · Default" }],
  },
  {
    slug: "xunfei-ws",
    kind: "xunfei-ws",
    transport: "ws",
    brand: "iFlytek IAT",
    chineseName: "讯飞听写",
    protocol: "WS",
    iconKey: "iflytek",
    defaultBaseUrl: "wss://iat-api.xfyun.cn",
    requiresBaseUrl: false,
    // Each id maps onto `business.domain` in the IAT request.
    defaultModels: [
      { id: "iat", name: "通用听写 (IAT)" },
      { id: "iat-niche-chs", name: "方言识别 (Niche)" },
      { id: "medical", name: "医疗领域" },
    ],
  },
  {
    slug: "volcengine-ws",
    kind: "volcengine-ws",
    transport: "ws",
    brand: "Volcengine / Doubao",
    chineseName: "火山引擎 / 豆包",
    protocol: "WS",
    iconKey: "volcengine",
    defaultBaseUrl: "wss://openspeech.bytedance.com",
    requiresBaseUrl: false,
    // model_name is hardcoded to "bigmodel" inside the provider —
    // tier selection is via `extra.resource_id`. The row here is a
    // UI label / activation anchor only.
    defaultModels: [
      { id: "bigmodel", name: "豆包流式语音识别 (BigModel)" },
    ],
  },
]

const PRESET_BY_SLUG: Record<string, SttKindPreset> = Object.fromEntries(
  STT_PRESETS.map((p) => [p.slug, p]),
)

export function findPreset(slug: string): SttKindPreset | undefined {
  return PRESET_BY_SLUG[slug]
}

/**
 * Resolve a slug for an already-saved provider. When two presets share
 * a kind (chat-completions-asr generic vs DashScope), pick by base URL
 * hostname; fall back to the first preset with the kind so the
 * dropdown always has a valid selection.
 */
export function presetSlugFromProvider(
  kind: SttProviderKind,
  baseUrl: string,
): string {
  if (
    kind === "openai-chat-completions-asr" &&
    baseUrl.toLowerCase().includes("dashscope")
  ) {
    return "dashscope"
  }
  if (PRESET_BY_SLUG[kind]) return kind
  const byKind = STT_PRESETS.find((p) => p.kind === kind)
  return byKind?.slug ?? kind
}
