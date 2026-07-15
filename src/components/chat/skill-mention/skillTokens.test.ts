import { describe, expect, it } from "vitest"

import {
  formatSkillInsertion,
  parseSkillMentions,
  skillMatchesQuery,
  skillMentionMeta,
} from "./skillTokens"

describe("Data Analytics skill mention", () => {
  it("is exposed as an analysis capability with localized search terms", () => {
    expect(skillMentionMeta("ha-data-analytics")).toMatchObject({
      iconKind: "analytics",
      group: "analysis",
      labelKey: "chat.skillMention.labels.dataAnalytics",
    })
    expect(skillMatchesQuery("ha-data-analytics", "数据")).toBe(true)
    expect(skillMatchesQuery("ha-data-analytics", "kpi")).toBe(true)
  })

  it("round-trips through the same mention token as Office skills", () => {
    const token = formatSkillInsertion("ha-data-analytics", "数据分析")
    expect(parseSkillMentions(token)).toEqual([
      {
        start: 0,
        end: token.length,
        raw: token,
        name: "ha-data-analytics",
        label: "数据分析",
      },
    ])
  })
})
