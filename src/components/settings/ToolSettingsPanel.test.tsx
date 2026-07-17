import { describe, expect, it } from "vitest"

import { RESET_SECTION_BY_TAB } from "./toolSettingsReset"

describe("ToolSettingsPanel reset targets", () => {
  it("maps every tool tab to one stable backend section", () => {
    expect(RESET_SECTION_BY_TAB).toEqual({
      general: "general",
      webSearch: "web_search",
      webFetch: "web_fetch",
      imageGenerate: "image_generate",
      audioGenerate: "audio_generate",
      canvas: "canvas",
      asyncTools: "async_tools",
      issueReporting: "issue_reporting",
    })
  })
})
