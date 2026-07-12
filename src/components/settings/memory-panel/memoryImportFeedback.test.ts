import type { TFunction } from "i18next"
import { describe, expect, test } from "vitest"
import {
  formatMemoryImportPreviewDiagnostics,
  formatMemoryImportPreviewIssueMessage,
  formatMemoryImportOperationError,
  formatMemoryImportPromptLoadError,
  formatMemoryImportResultError,
  formatMemoryImportScopeLabel,
  formatMemoryImportScopeSummaryKey,
  memoryImportDiagnosticText,
  memoryImportErrorDetail,
  memoryImportPreviewDescription,
  memoryImportPreviewCanApply,
  memoryImportPreviewDedupStatusLabel,
  memoryImportPreviewIssueMessages,
  memoryImportPreviewIsCurrent,
  memoryImportPreviewSampleWindowLabel,
  memoryImportPreviewStatusLabel,
  memoryImportSortedCountEntries,
} from "./memoryImportFeedback"
import type { MemoryImportPreview } from "./types"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

const t = ((
  key: string,
  fallback?: string | Record<string, unknown>,
  options?: Record<string, unknown>,
) => {
  const text = typeof fallback === "string" ? fallback : key
  return interpolate(text, typeof fallback === "object" ? fallback : options)
}) as unknown as TFunction

const zhT = ((
  key: string,
  fallback?: string | Record<string, unknown>,
  options?: Record<string, unknown>,
) => {
  const translations: Record<string, string> = {
    "settings.memoryImportIssueParseError": "无法解析这份记忆导入内容",
    "settings.memoryImportIssueNoImportableEntries": "没有发现可导入的记忆。",
    "settings.memoryImportIssueDedupPreviewFailed": "无法预估重复项",
    "settings.memoryImportFromAIPromptLoadFailed": "无法加载导入提示词。",
    "settings.memoryImportFromAIPromptLoadError": "无法加载导入提示词：{{error}}",
    "settings.memoryImportFromAIPromptCopyFailed": "复制导入提示词失败。",
    "settings.memoryImportFromAIPromptCopyError": "复制导入提示词失败：{{error}}",
    "settings.memoryImportPreviewFailed": "无法预览记忆导入。",
    "settings.memoryImportPreviewError": "无法预览记忆导入：{{error}}",
    "settings.memoryImportApplyFailed": "无法导入记忆。",
    "settings.memoryImportApplyError": "无法导入记忆：{{error}}",
    "settings.memoryImportCopyDiagnosticsFailed": "复制导入预览诊断失败。",
    "settings.memoryImportCopyDiagnosticsError": "复制导入预览诊断失败：{{error}}",
    "settings.memoryImportFirstError": "首条失败项：{{error}}",
    "settings.memoryImportPreviewTitle": "导入预览",
    "settings.memoryImportPreviewReport.title": "记忆导入预览",
    "settings.memoryImportPreviewReport.source": "来源",
    "settings.memoryImportPreviewReport.generated": "生成时间",
    "settings.memoryImportPreviewReport.valid": "有效",
    "settings.memoryImportPreviewReport.format": "格式",
    "settings.memoryImportPreviewReport.candidates": "候选",
    "settings.memoryImportPreviewReport.samplesIncluded": "包含样例",
    "settings.memoryImportPreviewReport.dedupChecked": "已检查去重",
    "settings.memoryImportPreviewReport.dedupStatus": "去重状态",
    "settings.memoryImportPreviewReport.yes": "是",
    "settings.memoryImportPreviewReport.no": "否",
    "settings.memoryImportPreviewReport.issues": "问题",
    "settings.memoryImportPreviewReport.samples": "示例",
    "settings.memoryImportPreviewReport.none": "（无）",
    "settings.memoryImportPreviewReady": "可导入",
    "settings.memoryImportPreviewBlocked": "不可导入",
    "settings.memoryImportPreviewNoSamples": "没有可展示的预览样例",
    "settings.memoryImportPreviewSamplesShown": "显示 {{visible}} / {{total}} 条样例",
    "settings.memoryImportPreviewDedupChecked": "重复估算已完成",
    "settings.memoryImportPreviewDedupUnavailable": "重复估算不可用",
    "settings.memoryImportPreviewDedupNotChecked": "未检查重复估算",
    "settings.memoryScopeGlobal": "全局",
    "settings.memoryScopeAgent": "Agent",
    "settings.memoryScopeProject": "项目",
  }
  const text = translations[key] ?? (typeof fallback === "string" ? fallback : key)
  return interpolate(text, typeof fallback === "object" ? fallback : options)
}) as unknown as TFunction

describe("formatMemoryImportPreviewDiagnostics", () => {
  test("includes parser, dedup, issue, and sample diagnostics", () => {
    const preview: MemoryImportPreview = {
      valid: true,
      format: "auto",
      candidateCount: 2,
      dedupChecked: true,
      likelyNewCount: 1,
      likelyMergeCount: 1,
      likelyDuplicateCount: 0,
      byType: { user: 1, project: 1 },
      byScope: { global: 1, "project:hope": 1 },
      issues: [{ code: "dedup_preview_failed", message: "partial estimate" }],
      samples: [
        {
          memoryType: "user",
          scope: { kind: "global" },
          contentPreview: "Prefers concise Chinese replies.",
          tags: ["profile"],
          dedupStatus: "new",
        },
        {
          memoryType: "project",
          scope: { kind: "project", id: "hope" },
          contentPreview: "Project uses review-first memory.",
          tags: ["project"],
          dedupStatus: "merge",
          dedupExistingId: 42,
          dedupExistingPreview: "Project uses structured memory.",
          dedupScore: 0.015,
        },
      ],
    }

    const markdown = formatMemoryImportPreviewDiagnostics(t, preview, "memory.md")

    expect(markdown).toContain("# Memory Import Preview")
    expect(markdown).toContain("- Source: memory.md")
    expect(markdown).toContain("- Valid: yes")
    expect(markdown).toContain("- Format: auto")
    expect(markdown).toContain("- Candidates: 2")
    expect(markdown).toContain("- Samples included: 2")
    expect(markdown).toContain("- Dedup status: Duplicate estimate ready")
    expect(markdown).toContain("- Estimated import: 2")
    expect(markdown).toContain("- Likely merge: 1")
    expect(markdown).toContain("- dedup_preview_failed: Could not estimate duplicates: partial estimate")
    expect(markdown).toContain("Prefers concise Chinese replies.")
    expect(markdown).toContain("project | Project: hope | merge")
    expect(markdown).toContain("Existing #42, score=0.0150: Project uses structured memory.")
  })

  test("formats known import preview issues for UI toasts and copied diagnostics", () => {
    expect(
      formatMemoryImportPreviewIssueMessage(zhT, {
        code: "parse_error",
        message: "Unsupported format: csv",
      }),
    ).toBe("无法解析这份记忆导入内容: Unsupported format: csv")
    expect(
      formatMemoryImportPreviewIssueMessage(zhT, {
        code: "no_importable_entries",
        message: "No importable memories found.",
      }),
    ).toBe("没有发现可导入的记忆。")
    expect(
      formatMemoryImportPreviewIssueMessage(zhT, {
        code: "dedup_preview_failed",
        message: "Could not estimate duplicates: vector index unavailable",
      }),
    ).toBe("无法预估重复项: vector index unavailable")

    const markdown = formatMemoryImportPreviewDiagnostics(
      zhT,
      {
        valid: false,
        format: "auto",
        candidateCount: 0,
        dedupChecked: false,
        likelyNewCount: 0,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: {},
        byScope: {},
        issues: [
          {
            code: "no_importable_entries",
            message: "No importable memories found.",
          },
        ],
        samples: [],
      },
      "memory.md",
    )

    expect(markdown).toContain("# 记忆导入预览")
    expect(markdown).toContain("- 来源: memory.md")
    expect(markdown).toContain("- 有效: 否")
    expect(markdown).toContain("- 格式: auto")
    expect(markdown).toContain("- 候选: 0")
    expect(markdown).toContain("- 包含样例: 0")
    expect(markdown).toContain("- 已检查去重: 否")
    expect(markdown).toContain("- 去重状态: 未检查重复估算")
    expect(markdown).toContain("## 问题")
    expect(markdown).toContain("- no_importable_entries: 没有发现可导入的记忆。")
    expect(markdown).toContain("## 示例")
    expect(markdown).toContain("- （无）")
    expect(markdown).not.toContain("# Memory Import Preview")
    expect(markdown).not.toContain("- no_importable_entries: No importable memories found.")
  })

  test("formats scope summary keys for UI chips and copied diagnostics", () => {
    expect(formatMemoryImportScopeLabel(zhT, { kind: "global" })).toBe("全局")
    expect(formatMemoryImportScopeLabel(zhT, { kind: "agent", id: "default" })).toBe(
      "Agent: default",
    )
    expect(formatMemoryImportScopeLabel(zhT, { kind: "project", id: "hope" })).toBe("项目: hope")
    expect(formatMemoryImportScopeSummaryKey(zhT, "global")).toBe("全局")
    expect(formatMemoryImportScopeSummaryKey(zhT, "agent:default")).toBe("Agent: default")
    expect(formatMemoryImportScopeSummaryKey(zhT, "project:hope")).toBe("项目: hope")
    expect(formatMemoryImportScopeSummaryKey(zhT, "workspace:custom")).toBe("workspace:custom")

    const markdown = formatMemoryImportPreviewDiagnostics(
      zhT,
      {
        valid: true,
        format: "auto",
        candidateCount: 2,
        dedupChecked: false,
        likelyNewCount: 2,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: {},
        byScope: { global: 1, "project:hope": 1 },
        issues: [],
        samples: [],
      },
      "memory.md",
    )

    expect(markdown).toContain("- 全局: 1")
    expect(markdown).toContain("- 项目: hope: 1")
    expect(markdown).not.toContain("- project:hope: 1")
  })

  test("sorts count summaries by count then stable raw key", () => {
    expect(
      memoryImportSortedCountEntries({
        user: 1,
        reference: 2,
        feedback: 2,
        project: 1,
      }),
    ).toEqual([
      ["feedback", 2],
      ["reference", 2],
      ["project", 1],
      ["user", 1],
    ])

    const markdown = formatMemoryImportPreviewDiagnostics(
      t,
      {
        valid: true,
        format: "json",
        candidateCount: 6,
        dedupChecked: false,
        likelyNewCount: 6,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: {
          user: 1,
          reference: 2,
          feedback: 2,
          project: 1,
        },
        byScope: {
          "project:z": 1,
          global: 1,
          "project:a": 2,
          "agent:default": 2,
        },
        issues: [],
        samples: [],
      },
      "memory.json",
    )

    expectInOrder(markdown, "- feedback: 2", "- reference: 2", "- project: 1", "- user: 1")
    expectInOrder(
      markdown,
      "- Agent: default: 2",
      "- Project: a: 2",
      "- Global: 1",
      "- Project: z: 1",
    )
  })

  test("projects preview issue messages for inline preview panels", () => {
    const messages = memoryImportPreviewIssueMessages(
      zhT,
      {
        issues: [
          { code: "no_importable_entries", message: "No importable memories found." },
          {
            code: "dedup_preview_failed",
            message: "Could not estimate duplicates: vector index unavailable",
          },
          { code: "parse_error", message: "Unsupported format: csv" },
        ],
      },
      2,
    )

    expect(messages).toEqual([
      "没有发现可导入的记忆。",
      "无法预估重复项: vector index unavailable",
    ])
  })

  test("keeps invalid current previews visible but not applyable", () => {
    expect(memoryImportPreviewIsCurrent({ valid: false }, true)).toBe(true)
    expect(memoryImportPreviewCanApply({ valid: false }, true)).toBe(false)
    expect(memoryImportPreviewCanApply({ valid: true }, false)).toBe(false)
    expect(memoryImportPreviewCanApply({ valid: true }, true)).toBe(true)
    expect(memoryImportPreviewIsCurrent(null, true)).toBe(false)
  })

  test("labels import preview status for visible UI chips", () => {
    expect(memoryImportPreviewStatusLabel(t, { valid: true })).toBe("Ready to import")
    expect(memoryImportPreviewStatusLabel(t, { valid: false })).toBe("Cannot import")
    expect(memoryImportPreviewStatusLabel(zhT, { valid: true })).toBe("可导入")
    expect(memoryImportPreviewStatusLabel(zhT, { valid: false })).toBe("不可导入")
  })

  test("labels sample windows only when empty or truncated", () => {
    expect(memoryImportPreviewSampleWindowLabel(t, 0, 0)).toBe("No preview samples")
    expect(memoryImportPreviewSampleWindowLabel(t, 3, 3)).toBeNull()
    expect(memoryImportPreviewSampleWindowLabel(t, 8, 4)).toBe("Showing 4 of 8 samples")
    expect(memoryImportPreviewSampleWindowLabel(zhT, 8, 4)).toBe("显示 4 / 8 条样例")
  })

  test("labels dedup preview status separately from the boolean", () => {
    expect(memoryImportPreviewDedupStatusLabel(t, { dedupChecked: true, issues: [] })).toBe(
      "Duplicate estimate ready",
    )
    expect(memoryImportPreviewDedupStatusLabel(t, { dedupChecked: false, issues: [] })).toBe(
      "Duplicate estimate not checked",
    )
    expect(
      memoryImportPreviewDedupStatusLabel(zhT, {
        dedupChecked: false,
        issues: [{ code: "dedup_preview_failed", message: "Could not estimate duplicates: index" }],
      }),
    ).toBe("重复估算不可用")

    const markdown = formatMemoryImportPreviewDiagnostics(
      zhT,
      {
        valid: true,
        format: "auto",
        candidateCount: 1,
        dedupChecked: false,
        likelyNewCount: 1,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: { user: 1 },
        byScope: { global: 1 },
        issues: [
          {
            code: "dedup_preview_failed",
            message: "Could not estimate duplicates: vector index unavailable",
          },
        ],
        samples: [],
      },
      "memory.md",
    )

    expect(markdown).toContain("- 去重状态: 重复估算不可用")
    expect(markdown).toContain("- dedup_preview_failed: 无法预估重复项: vector index unavailable")
  })

  test("keeps dedup preview failure visible in post-import descriptions", () => {
    expect(
      memoryImportPreviewDescription(zhT, {
        valid: true,
        format: "auto",
        candidateCount: 1,
        dedupChecked: false,
        likelyNewCount: 1,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: { user: 1 },
        byScope: { global: 1 },
        issues: [
          {
            code: "dedup_preview_failed",
            message: "Could not estimate duplicates: vector index unavailable",
          },
        ],
        samples: [],
      }),
    ).toBe("导入预览: 重复估算不可用")

    expect(
      memoryImportPreviewDescription(zhT, {
        valid: true,
        format: "auto",
        candidateCount: 1,
        dedupChecked: false,
        likelyNewCount: 1,
        likelyMergeCount: 0,
        likelyDuplicateCount: 0,
        byType: { user: 1 },
        byScope: { global: 1 },
        issues: [],
        samples: [],
      }),
    ).toBeUndefined()
  })

  test("redacts sensitive import diagnostics", () => {
    const diagnostic = memoryImportDiagnosticText(
      "preview failed https://api.example.test/import?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret passphrase=backup-secret",
    )

    expect(diagnostic).toContain("token=[redacted]")
    expect(diagnostic).toContain("Authorization: Bearer [redacted]")
    expect(diagnostic).toContain("api_key=[redacted]")
    expect(diagnostic).toContain("passphrase=[redacted]")
    expect(diagnostic).not.toContain("query-secret")
    expect(diagnostic).not.toContain("bearer-secret")
    expect(diagnostic).not.toContain("sk-live-secret")
    expect(diagnostic).not.toContain("backup-secret")
  })

  test("formats import result errors without exposing sensitive detail", () => {
    expect(formatMemoryImportResultError(zhT, "invalid scope: project")).toBe(
      "首条失败项：invalid scope: project",
    )
    expect(formatMemoryImportResultError(zhT, "invalid row token=row-secret")).toBe(
      "首条失败项：invalid row token=[redacted]",
    )
    expect(formatMemoryImportResultError(t, "database is locked")).toBe(
      "First failed item: database is locked",
    )
    expect(formatMemoryImportResultError(zhT, "   ")).toBeNull()
    expect(formatMemoryImportResultError(zhT, null)).toBeNull()
  })

  test("formats prompt-load errors without exposing naked exceptions", () => {
    expect(memoryImportErrorDetail(new Error("IPC unavailable"))).toBe("IPC unavailable")
    expect(memoryImportErrorDetail("IPC unavailable api_key=secret-key")).toBe(
      "IPC unavailable api_key=[redacted]",
    )
    expect(memoryImportErrorDetail("  timeout  ")).toBe("timeout")
    expect(memoryImportErrorDetail("   ")).toBeNull()
    expect(memoryImportErrorDetail(null)).toBeNull()

    expect(formatMemoryImportPromptLoadError(zhT, new Error("IPC unavailable"))).toBe(
      "无法加载导入提示词：IPC unavailable",
    )
    expect(formatMemoryImportPromptLoadError(t, "timeout")).toBe(
      "Failed to load import prompt: timeout",
    )
    expect(formatMemoryImportPromptLoadError(zhT, "   ")).toBe("无法加载导入提示词。")
  })

  test("formats preview, apply, prompt-copy, and diagnostics-copy errors with action context", () => {
    expect(
      formatMemoryImportOperationError(zhT, "preview", new Error("invalid json token=secret")),
    ).toBe("无法预览记忆导入：invalid json token=[redacted]")
    expect(formatMemoryImportOperationError(zhT, "apply", "database locked")).toBe(
      "无法导入记忆：database locked",
    )
    expect(formatMemoryImportOperationError(zhT, "copyPrompt", "clipboard denied")).toBe(
      "复制导入提示词失败：clipboard denied",
    )
    expect(formatMemoryImportOperationError(zhT, "copyDiagnostics", "clipboard denied")).toBe(
      "复制导入预览诊断失败：clipboard denied",
    )
    expect(formatMemoryImportOperationError(t, "preview", "timeout")).toBe(
      "Failed to preview memory import: timeout",
    )
    expect(formatMemoryImportOperationError(t, "copyDiagnostics", null)).toBe(
      "Failed to copy memory import diagnostics.",
    )
    expect(formatMemoryImportOperationError(t, "copyPrompt", null)).toBe(
      "Failed to copy import prompt.",
    )
    expect(formatMemoryImportOperationError(t, "apply", "   ")).toBe(
      "Failed to import memories.",
    )
  })
})

function expectInOrder(text: string, ...needles: string[]) {
  let previousIndex = -1
  for (const needle of needles) {
    const nextIndex = text.indexOf(needle)
    expect(nextIndex).toBeGreaterThan(previousIndex)
    previousIndex = nextIndex
  }
}
