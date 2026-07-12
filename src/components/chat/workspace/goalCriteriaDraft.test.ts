import { describe, expect, it } from "vitest"
import { parseGoalCriteriaDraft } from "./goalCriteriaDraft"

describe("parseGoalCriteriaDraft", () => {
  it("matches the backend criteria grouping semantics for draft previews", () => {
    const items = parseGoalCriteriaDraft(`
      [required]
      - pass targeted checks
      [optional] polish copy
      follow-up: browser screenshot smoke
      - export to roadmap
      [follow-up]
      - migrate into roadmap
    `)

    expect(items).toEqual([
      { id: "criterion-1", text: "pass targeted checks", kind: "required" },
      { id: "criterion-2", text: "polish copy", kind: "optional" },
      { id: "criterion-3", text: "browser screenshot smoke", kind: "follow_up" },
      { id: "criterion-4", text: "export to roadmap", kind: "required" },
      { id: "criterion-5", text: "migrate into roadmap", kind: "follow_up" },
    ])
  })

  it("matches backend prefix variants used by localized goal drafts", () => {
    const items = parseGoalCriteriaDraft(`
      \u5fc5\u987b\uff1a \u2610 \u8dd1\u5b8c\u9488\u5bf9\u6027\u68c0\u67e5
      1) optional: polish UX copy
      * [follow-up] migrate notes to roadmap
    `)

    expect(items).toEqual([
      { id: "criterion-1", text: "\u8dd1\u5b8c\u9488\u5bf9\u6027\u68c0\u67e5", kind: "required" },
      { id: "criterion-2", text: "polish UX copy", kind: "optional" },
      { id: "criterion-3", text: "migrate notes to roadmap", kind: "follow_up" },
    ])
  })
})
