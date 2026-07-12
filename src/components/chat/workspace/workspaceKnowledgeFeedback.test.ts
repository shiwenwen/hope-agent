import { describe, expect, it } from "vitest"

import {
  workspaceKnowledgeDiagnosticText,
  workspaceKnowledgeErrorDetail,
} from "./workspaceKnowledgeFeedback"

describe("workspace knowledge feedback", () => {
  it("formats knowledge attachment read failures with redacted detail", () => {
    expect(workspaceKnowledgeErrorDetail(new Error("sqlite busy"))).toBe("sqlite busy")
    expect(workspaceKnowledgeErrorDetail("   ")).toBeNull()
    expect(
      workspaceKnowledgeDiagnosticText(
        "load failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "load failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
  })
})
