import type { SettingsResetSection } from "./settingsReset"

export type ToolTab =
  | "general"
  | "webSearch"
  | "webFetch"
  | "mediaGenerate"
  | "canvas"
  | "asyncTools"
  | "issueReporting"

export const RESET_SECTION_BY_TAB: Record<ToolTab, SettingsResetSection> = {
  general: "general",
  webSearch: "web_search",
  webFetch: "web_fetch",
  mediaGenerate: "media_gen",
  canvas: "canvas",
  asyncTools: "async_tools",
  issueReporting: "issue_reporting",
}
