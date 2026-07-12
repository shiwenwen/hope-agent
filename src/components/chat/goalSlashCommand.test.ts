import { describe, expect, test } from "vitest"

import {
  goalSlashCommandDisplay,
  parseGoalObjectiveAndCriteria,
  parseGoalUpsertSlashCommand,
} from "./goalSlashCommand"

describe("goal slash command parsing", () => {
  test("treats exact control words as commands", () => {
    expect(parseGoalUpsertSlashCommand("/goal status")).toBeNull()
    expect(goalSlashCommandDisplay("/goal status")).toEqual({
      content: "Show active goal",
      mode: "goal",
    })
  })

  test("treats longer text starting with a control word as a goal objective", () => {
    const text = "status should render as objective"

    expect(parseGoalUpsertSlashCommand(`/goal ${text}`)).toBe(text)
    expect(goalSlashCommandDisplay(`/goal ${text}`)).toEqual({
      content: text,
      mode: "goal",
    })
  })

  test("splits objective and criteria for first-turn goal creation", () => {
    expect(
      parseGoalObjectiveAndCriteria("Ship Goal v3 --criteria status card stays concise"),
    ).toEqual({
      objective: "Ship Goal v3",
      completionCriteria: "status card stays concise",
    })
    expect(parseGoalObjectiveAndCriteria("完成文档更新 完成标准：通过 typecheck")).toEqual({
      objective: "完成文档更新",
      completionCriteria: "通过 typecheck",
    })
  })
})
