// Vendor-kind display metadata for the media-generation settings surfaces.
// Brand names are data (not i18n); icon keys map into ProviderIcon's
// ICON_MAP — unmapped kinds fall back to its generic icon.

import type { MediaVendorKind } from "./types"

export const VENDOR_DISPLAY_NAME: Record<MediaVendorKind, string> = {
  openai: "OpenAI",
  google: "Google",
  fal: "Fal",
  minimax: "MiniMax",
  siliconflow: "SiliconFlow",
  zhipu: "ZhipuAI",
  tongyi: "Tongyi Wanxiang",
  elevenlabs: "ElevenLabs",
  stepfun: "StepFun",
  volcengine: "Volcengine Ark",
  hunyuan: "Tencent Hunyuan",
  together: "Together AI",
  xai: "xAI",
  recraft: "Recraft",
  qianfan: "Baidu Qianfan",
  sensenova: "SenseNova",
  cartesia: "Cartesia",
  deepgram: "Deepgram",
  fishaudio: "Fish Audio",
  hume: "Hume AI",
  bfl: "Black Forest Labs",
  stability: "Stability AI",
  replicate: "Replicate",
  kling: "Kling",
  iflytek: "iFlytek Spark",
  "volcengine-tts": "Doubao Speech",
  "openai-compatible": "OpenAI Compatible",
}

/** ProviderIcon key per vendor kind. `openai-compatible` is intentionally
 *  unmapped so it renders the generic settings glyph (distinct from the
 *  OpenAI brand mark). */
export const VENDOR_ICON_KEY: Partial<Record<MediaVendorKind, string>> = {
  openai: "openai",
  google: "google-gemini",
  fal: "fal",
  minimax: "minimax",
  siliconflow: "siliconflow",
  zhipu: "zhipu",
  tongyi: "qwen",
  elevenlabs: "elevenlabs",
  stepfun: "stepfun",
  volcengine: "volcengine",
  hunyuan: "tencent",
  together: "together",
  xai: "xai",
  recraft: "recraft",
  qianfan: "qianfan",
  sensenova: "sensenova",
  cartesia: "cartesia",
  deepgram: "deepgram",
  fishaudio: "fishaudio",
  hume: "hume",
  bfl: "bfl",
  stability: "stability",
  replicate: "replicate",
  kling: "kling",
  iflytek: "iflytek",
  "volcengine-tts": "volcengine",
}

/** Grouping for the "add provider" template grid. Vendors serving both
 *  modalities appear once, under `both`. */
export type VendorGroup = "image" | "audio" | "both" | "custom"

export const VENDOR_GROUP: Record<MediaVendorKind, VendorGroup> = {
  openai: "both",
  google: "image",
  fal: "image",
  siliconflow: "image",
  zhipu: "image",
  tongyi: "image",
  elevenlabs: "audio",
  stepfun: "both",
  volcengine: "image",
  hunyuan: "image",
  together: "image",
  xai: "image",
  recraft: "image",
  qianfan: "image",
  sensenova: "image",
  cartesia: "audio",
  deepgram: "audio",
  fishaudio: "audio",
  hume: "audio",
  minimax: "both",
  bfl: "image",
  stability: "both",
  replicate: "image",
  kling: "both",
  iflytek: "image",
  "volcengine-tts": "audio",
  "openai-compatible": "custom",
}

export const VENDOR_GROUP_ORDER: VendorGroup[] = ["both", "image", "audio", "custom"]

export const VENDOR_GROUP_LABEL_KEY: Record<VendorGroup, string> = {
  both: "settings.mediaModels.groupBoth",
  image: "settings.mediaModels.groupImage",
  audio: "settings.mediaModels.groupAudio",
  custom: "settings.mediaModels.groupCustom",
}
