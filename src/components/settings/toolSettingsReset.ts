import type { SettingsResetSection } from "./settingsReset"

export type ToolTab =
  | "general"
  | "webSearch"
  | "webFetch"
  | "imageGenerate"
  | "audioGenerate"
  | "canvas"
  | "asyncTools"
  | "issueReporting"

export const RESET_SECTION_BY_TAB: Record<ToolTab, SettingsResetSection> = {
  general: "general",
  webSearch: "web_search",
  webFetch: "web_fetch",
  imageGenerate: "image_generate",
  audioGenerate: "audio_generate",
  canvas: "canvas",
  asyncTools: "async_tools",
  issueReporting: "issue_reporting",
}
