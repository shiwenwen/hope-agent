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
}
