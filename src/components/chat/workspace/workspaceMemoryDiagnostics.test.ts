import { describe, expect, it } from "vitest"

import type { Message } from "@/types/chat"
import {
  buildWorkspaceMemoryDiagnostics,
  formatWorkspaceMemoryDiagnosticsMarkdown,
  workspaceMemoryDiagnosticText,
  workspaceMemoryDiagnosticsCopyErrorDetail,
  workspaceMemoryDiagnosticsCopyErrorToast,
} from "./workspaceMemoryDiagnostics"

function assistant(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "ok",
    timestamp: "2026-07-07T00:00:00.000Z",
    ...patch,
  }
}

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("workspace memory diagnostics", () => {
  it("aggregates refs and retrieval layers across assistant turns", () => {
    const diagnostics = buildWorkspaceMemoryDiagnostics([
      { role: "user", content: "hi" },
      assistant({
        usedMemoryRefs: [
          {
            kind: "memory",
            id: "m1",
            origin: "active_memory",
            role: "selected",
            preview: "private preference",
          },
          {
            kind: "claim",
            id: "c1",
            origin: "graph",
            role: "candidate",
            preview: "private candidate",
          },
        ],
        retrievalPlanner: {
          status: "partial",
          totalRefs: 2,
          layers: [
            {
              layer: "active_memory",
              status: "used",
              refCount: 1,
              selectedCount: 1,
              latencyMs: 42,
              cached: true,
            },
            {
              layer: "knowledge",
              status: "skipped",
              refCount: 0,
              skippedReason: "timeout",
              latencyMs: 4500,
            },
          ],
        },
      }),
      assistant({
        retrievalPlanner: {
          status: "degraded",
          totalRefs: 0,
          layers: [
            {
              layer: "knowledge",
              status: "skipped",
              refCount: 0,
              skippedReason: "timeout",
              latencyMs: 4100,
            },
          ],
        },
      }),
    ])

    expect(diagnostics.turns).toBe(2)
    expect(diagnostics.refCount).toBe(2)
    expect(diagnostics.contextRefCount).toBe(1)
    expect(diagnostics.candidateRefCount).toBe(1)
    expect(diagnostics.statusCounts).toMatchObject({ degraded: 1, partial: 1 })
    expect(diagnostics.kindCounts).toMatchObject({ claim: 1, memory: 1 })
    expect(diagnostics.originCounts).toMatchObject({ active_memory: 1, graph: 1 })
    expect(diagnostics.recentTurns).toEqual([
      expect.objectContaining({
        index: 1,
        status: "partial",
        contextRefCount: 1,
        candidateRefCount: 1,
        degraded: true,
      }),
      expect.objectContaining({
        index: 2,
        status: "degraded",
        contextRefCount: 0,
        candidateRefCount: 0,
        degraded: true,
      }),
    ])
    expect(diagnostics.layers.find((layer) => layer.layer === "knowledge")).toMatchObject({
      turns: 2,
      skipped: 2,
      latencyMs: 4500,
    })
    expect(diagnostics.degradedLayers[0]).toMatchObject({
      layer: "knowledge",
      status: "skipped",
      reason: "timeout",
      count: 2,
      latestLatencyMs: 4100,
    })
  })

  it("formats bounded support diagnostics without leaking memory previews", () => {
    const diagnostics = buildWorkspaceMemoryDiagnostics([
      assistant({
        usedMemoryRefs: [
          {
            kind: "memory",
            id: "m-secret",
            origin: "static_memory",
            role: "injected",
            preview: "Secret user preference",
          },
        ],
        retrievalPlanner: {
          status: "used",
          totalRefs: 1,
          layers: [{ layer: "static_memory", status: "used", refCount: 1 }],
        },
      }),
    ])

    const markdown = formatWorkspaceMemoryDiagnosticsMarkdown(diagnostics)

    expect(markdown).toContain("# Workspace Memory Diagnostics")
    expect(markdown).toContain("- Memory refs: 1")
    expect(markdown).toContain("## Recent Turns")
    expect(markdown).toContain("- #1: used, context=1, candidates=0")
    expect(markdown).toContain("- static_memory: turns=1")
    expect(markdown).not.toContain("Secret user preference")
    expect(markdown).not.toContain("m-secret")
  })

  it("formats copy failure feedback with action context and redacted detail", () => {
    expect(workspaceMemoryDiagnosticsCopyErrorDetail(new Error("clipboard denied"))).toBe(
      "clipboard denied",
    )
    expect(workspaceMemoryDiagnosticsCopyErrorDetail("   ")).toBeNull()
    expect(
      workspaceMemoryDiagnosticText(
        "copy failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "copy failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      workspaceMemoryDiagnosticsCopyErrorDetail(
        "copy failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("copy failed password=[redacted] passphrase=[redacted]")

    const translations: Record<string, string> = {
      "workspace.memoryDiagnostics.copyFailed": "复制记忆诊断失败",
      "workspace.memoryDiagnostics.copyDetail": "详细信息：{{error}}",
    }
    const t = (key: string, defaultValue: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? defaultValue, options)

    expect(workspaceMemoryDiagnosticsCopyErrorToast(t, "clipboard denied")).toEqual({
      title: "复制记忆诊断失败",
      description: "详细信息：clipboard denied",
    })
    expect(workspaceMemoryDiagnosticsCopyErrorToast(t, "clipboard denied token=workspace-secret")).toEqual({
      title: "复制记忆诊断失败",
      description: "详细信息：clipboard denied token=[redacted]",
    })
    expect(workspaceMemoryDiagnosticsCopyErrorToast(t, null)).toEqual({
      title: "复制记忆诊断失败",
    })
  })
})
