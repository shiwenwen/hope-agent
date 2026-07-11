import { describe, expect, it } from "vitest"

import {
  knowledgeChatErrorDetail,
  knowledgeChatIssueDescription,
  knowledgeChatIssueTitle,
  knowledgeChatLoadIssue,
} from "./knowledgeChatFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge chat feedback", () => {
  it("extracts and redacts chat diagnostics", () => {
    expect(knowledgeChatErrorDetail(new Error("thread load failed"))).toBe("thread load failed")
    expect(
      knowledgeChatErrorDetail(
        "history failed Authorization: Bearer chat-secret token=query-secret api_key=session-secret",
      ),
    ).toBe(
      "history failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeChatErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeChatErrorDetail("   ")).toBeNull()
    expect(knowledgeChatErrorDetail(undefined)).toBeNull()
  })

  it("formats localized chat load issues", () => {
    const translations: Record<string, string> = {
      "knowledge.chatPanel.errors.loadThreads": "无法加载对话历史",
      "knowledge.chatPanel.errors.loadThread": "无法加载当前对话",
      "knowledge.chatPanel.errors.loadAgentConfig": "无法加载当前 Agent 对话设置",
      "knowledge.chatPanel.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    const historyIssue = knowledgeChatLoadIssue("loadThreads", "load token=history-secret")
    expect(knowledgeChatIssueTitle(historyIssue, t)).toBe("无法加载对话历史")
    expect(knowledgeChatIssueDescription(historyIssue, t)).toBe(
      "详细信息：load token=[redacted]",
    )

    const threadIssue = knowledgeChatLoadIssue("loadThread", null)
    expect(knowledgeChatIssueTitle(threadIssue, t)).toBe("无法加载当前对话")
    expect(knowledgeChatIssueDescription(threadIssue, t)).toBeNull()

    const agentConfigIssue = knowledgeChatLoadIssue(
      "loadAgentConfig",
      "config Authorization: Bearer agent-secret",
    )
    expect(knowledgeChatIssueTitle(agentConfigIssue, t)).toBe(
      "无法加载当前 Agent 对话设置",
    )
    expect(knowledgeChatIssueDescription(agentConfigIssue, t)).toBe(
      "详细信息：config Authorization: Bearer [redacted]",
    )
  })

  it("uses English fallback when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)
    const issue = knowledgeChatLoadIssue("loadModels", "provider denied")

    expect(knowledgeChatIssueTitle(issue, t)).toBe("Couldn't load chat models")
    expect(knowledgeChatIssueDescription(issue, t)).toBe("Details: provider denied")

    const agentConfigIssue = knowledgeChatLoadIssue("loadAgentConfig", "config denied")
    expect(knowledgeChatIssueTitle(agentConfigIssue, t)).toBe(
      "Couldn't load this agent's chat settings",
    )
  })
})
