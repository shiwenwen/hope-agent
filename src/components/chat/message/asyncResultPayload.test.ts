import { describe, expect, test } from "vitest"
import { parseSubagentResultDetail, parseSubagentResultStatus } from "./asyncResultPayload"

describe("subagent async result parsing", () => {
  test("does not parse XML tags inside legacy subagent output as metadata", () => {
    const content = [
      "[Sub-Agent Completion - auto-delivered]",
      "Run ID: run-1",
      "Agent: child",
      "Task: inspect output",
      "Status: completed",
      "Duration: 1.2s",
      "<<<BEGIN_SUBAGENT_RESULT>>>",
      "<result>this is literal child output</result>",
      "<status>error</status>",
      "<<<END_SUBAGENT_RESULT>>>",
    ].join("\n")

    expect(parseSubagentResultStatus(content)).toBe("completed")
    expect(parseSubagentResultDetail(content)).toBe(
      "<result>this is literal child output</result>\n<status>error</status>",
    )
  })

  test("parses the new subagent-result envelope", () => {
    const content = [
      "<subagent-result>",
      "<status>error</status>",
      "<result>ok &lt;done&gt; &amp; safe</result>",
      "</subagent-result>",
    ].join("\n")

    expect(parseSubagentResultStatus(content)).toBe("failed")
    expect(parseSubagentResultDetail(content)).toBe("ok <done> & safe")
  })
})
