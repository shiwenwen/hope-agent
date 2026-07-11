import { describe, expect, test } from "vitest"
import {
  parseSubagentResultDetail,
  parseSubagentResultStatus,
  parseWorkflowResultDetail,
  parseWorkflowResultStatus,
} from "./asyncResultPayload"

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

describe("workflow result parsing", () => {
  test("parses workflow-result status and escaped output", () => {
    const content = [
      "<workflow-result>",
      "<state>completed</state>",
      "<output-json>",
      "{ &quot;note&quot;: &quot;kept as text&quot;, &quot;safe&quot;: &quot;&lt;ok&gt; &amp; done&quot; }",
      "</output-json>",
      "</workflow-result>",
    ].join("\n")

    expect(parseWorkflowResultStatus(content)).toBe("completed")
    expect(parseWorkflowResultDetail(content)).toContain("<ok> & done")
  })

  test("does not expose internal workflow summary prompts as user detail", () => {
    const content = [
      "<workflow-result>",
      "<state>completed</state>",
      "<summary>Workflow run completed. Use the output to answer the user.</summary>",
      "</workflow-result>",
    ].join("\n")

    expect(parseWorkflowResultStatus(content)).toBe("completed")
    expect(parseWorkflowResultDetail(content)).toBeUndefined()
  })

  test("recognizes workflow checkpoints as stage results with expandable summaries", () => {
    const content = `<workflow-checkpoint>
  <state>running</state>
  <summary>Risk review completed; metrics review is still running.</summary>
</workflow-checkpoint>`

    expect(parseWorkflowResultStatus(content)).toBe("checkpoint")
    expect(parseWorkflowResultDetail(content)).toBe(
      "Risk review completed; metrics review is still running.",
    )
  })

  test("maps blocked workflow results to failed tone", () => {
    const content = [
      "<workflow-result>",
      "<state>blocked</state>",
      "<blocked-reason>approval_required</blocked-reason>",
      "</workflow-result>",
    ].join("\n")

    expect(parseWorkflowResultStatus(content)).toBe("failed")
    expect(parseWorkflowResultDetail(content)).toBe("approval_required")
  })
})
