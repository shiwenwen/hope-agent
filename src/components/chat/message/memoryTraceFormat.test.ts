import { describe, expect, it } from "vitest"

import type { RetrievalPlannerLayerTrace, UsedMemoryRef } from "@/types/chat"

import {
  isMemoryCandidateRole,
  memoryKindLabel,
  memoryLocationLabel,
  memoryMetricLabels,
  memoryOriginLabel,
  memoryReasonText,
  memoryRoleLabel,
  memoryScopeLabel,
  memorySourceLabel,
  memoryTraceDiagnosticText,
  memoryTraceErrorDescription,
  memoryTraceErrorDetail,
  retrievalLayerDetailParts,
  retrievalLayerLabel,
  retrievalLayerReasonLabel,
  retrievalTraceStatusLabel,
  retrievalTraceSummary,
  retrievalTraceTitle,
  shouldRenderMemoryTracePanel,
} from "./memoryTraceFormat"

const t = (_key: string, fallbackOrOptions?: string | Record<string, unknown>): string => {
  if (typeof fallbackOrOptions === "string") return fallbackOrOptions
  if (fallbackOrOptions && "defaultValue" in fallbackOrOptions) {
    let text = String(fallbackOrOptions.defaultValue)
    if ("count" in fallbackOrOptions) {
      text = text.replace("{{count}}", String(fallbackOrOptions.count))
    }
    if ("id" in fallbackOrOptions) {
      text = text.replace("{{id}}", String(fallbackOrOptions.id))
    }
    if ("error" in fallbackOrOptions) {
      text = text.replace("{{error}}", String(fallbackOrOptions.error))
    }
    return text
  }
  if (fallbackOrOptions && "value" in fallbackOrOptions) {
    return `metric ${String(fallbackOrOptions.value)}`
  }
  return _key
}

describe("memoryTraceFormat", () => {
  it("formats retrieval layer details used by UI chips and markdown diagnostics", () => {
    const layer: RetrievalPlannerLayerTrace = {
      layer: "active_memory",
      status: "used",
      refCount: 3,
      injectedCount: 1,
      selectedCount: 1,
      candidateCount: 2,
      droppedCount: 4,
      latencyMs: 42,
      cached: true,
      skippedReason: "timeout",
    }

    expect(retrievalLayerDetailParts(layer, t)).toEqual([
      "used",
      "refs=3",
      "injected=1",
      "selected=1",
      "candidates=2",
      "dropped=4",
      "42ms",
      "cached",
      "timed out",
    ])
  })

  it("maps backend skipped reasons to user-readable labels", () => {
    expect(retrievalLayerReasonLabel("memory_off", t)).toBe("long-term memory is off")
    expect(retrievalLayerReasonLabel("agent_config_error", t)).toBe(
      "agent memory config unavailable",
    )
    expect(retrievalLayerReasonLabel("no_graph_neighbors", t)).toBe(
      "no related entity relationships",
    )
    expect(retrievalLayerReasonLabel("unknown_reason", t)).toBe("unknown_reason")
  })

  it("labels memory kinds, origins, and trace layers with English fallbacks", () => {
    expect(memoryKindLabel({ kind: "episode" }, t)).toBe("Experience")
    expect(memoryKindLabel({ kind: "procedure" }, t)).toBe("Workflow")
    expect(memoryOriginLabel("experience", t)).toBe("Experience memory")
    expect(memoryOriginLabel("graph", t)).toBe("Entity relationships")
    expect(memoryRoleLabel("selected", t)).toBe("Selected")
    expect(memoryRoleLabel("candidate", t)).toBe("Candidate")
    expect(memoryRoleLabel("considered", t)).toBe("Candidate")
    expect(memoryRoleLabel("injected", t)).toBe("Injected")
    expect(retrievalLayerLabel("experience", t)).toBe("Experience memory")
    expect(retrievalLayerLabel("graph", t)).toBe("Entity relationships")
  })

  it("labels raw scope ids as readable scope chips", () => {
    expect(memoryScopeLabel("global", t)).toBe("Global")
    expect(memoryScopeLabel("GLOBAL", t)).toBe("Global")
    expect(memoryScopeLabel("project", t)).toBe("Project")
    expect(memoryScopeLabel("project:hope-agent", t)).toBe("Project: hope-agent")
    expect(memoryScopeLabel("project:", t)).toBe("Project")
    expect(memoryScopeLabel("agent", t)).toBe("Agent")
    expect(memoryScopeLabel("agent:planner", t)).toBe("Agent: planner")
    expect(memoryScopeLabel("agent:", t)).toBe("Agent")
    expect(memoryScopeLabel("session", t)).toBe("Session")
    expect(memoryScopeLabel("session:turn-1", t)).toBe("Session: turn-1")
    expect(memoryScopeLabel("session:", t)).toBe("Session")
    expect(memoryScopeLabel("workspace:custom", t)).toBe("workspace:custom")
    expect(memoryScopeLabel("  ", t)).toBeNull()
    expect(memoryScopeLabel(undefined, t)).toBeNull()
  })

  it("formats source labels shared by UI chips and markdown diagnostics", () => {
    expect(memorySourceLabel({ sourceType: "claim", scope: "project:hope-agent" }, t)).toBe(
      "claim · Project: hope-agent",
    )
    expect(memorySourceLabel({ sourceType: "manual", scope: undefined }, t)).toBe("manual")
    expect(memorySourceLabel({ sourceType: undefined, scope: "global" }, t)).toBe("Global")
    expect(memorySourceLabel({ sourceType: undefined, scope: "  " }, t)).toBe("")
  })

  it("formats location labels only for positive finite coordinates", () => {
    expect(memoryLocationLabel({ kind: "knowledge", id: "kb:path", line: 12, col: 3 })).toBe(
      "L12:C3",
    )
    expect(memoryLocationLabel({ kind: "knowledge", id: "kb:path", line: 8 })).toBe("L8")
    expect(memoryLocationLabel({ kind: "knowledge", id: "kb:path", col: 4 })).toBe("C4")
    expect(
      memoryLocationLabel({
        kind: "knowledge",
        id: "kb:path",
        headingPath: "Project > Decisions",
        line: 12,
        col: 3,
      }),
    ).toBe("Project > Decisions · L12:C3")
    expect(
      memoryLocationLabel({
        kind: "knowledge",
        id: "kb:path",
        blockId: "decision-1",
      }),
    ).toBe("^decision-1")
    expect(
      memoryLocationLabel({
        kind: "knowledge",
        id: "kb:path",
        blockId: "^decision-1",
        headingPath: "Project",
      }),
    ).toBe("Project · ^decision-1")
    expect(memoryLocationLabel({ kind: "knowledge", id: "kb:path", line: 0, col: -1 })).toBeNull()
  })

  it("formats finite memory metrics and ignores invalid values", () => {
    const ref: UsedMemoryRef = {
      kind: "memory",
      id: "1",
      score: 0.42,
      confidence: 1.2,
      salience: Number.NaN,
    }

    expect(memoryMetricLabels(ref, t)).toEqual(["metric 42%", "metric 1.20"])
  })

  it("formats memory trace action error details without naked exceptions", () => {
    const zhT = (key: string, fallbackOrOptions?: string | Record<string, unknown>): string => {
      if (key === "chat.memoryTrace.errorDetail" && typeof fallbackOrOptions === "object") {
        return `详细信息：${String(fallbackOrOptions.error)}`
      }
      return t(key, fallbackOrOptions)
    }

    expect(memoryTraceErrorDetail(new Error("database locked"))).toBe("database locked")
    expect(memoryTraceErrorDetail("  timeout  ")).toBe("timeout")
    expect(
      memoryTraceDiagnosticText(
        "memory update failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "memory update failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryTraceErrorDetail(
        "claim archive failed passphrase=backup-secret password=db-secret",
      ),
    ).toBe("claim archive failed passphrase=[redacted] password=[redacted]")
    expect(memoryTraceErrorDetail("   ")).toBeNull()
    expect(memoryTraceErrorDetail(null)).toBeNull()
    expect(memoryTraceErrorDescription(new Error("database locked"), zhT)).toBe(
      "详细信息：database locked",
    )
    expect(memoryTraceErrorDescription("timeout", t)).toBe("Details: timeout")
    expect(memoryTraceErrorDescription("   ", t)).toBeUndefined()
  })

  it("explains generic candidates without implying they entered context", () => {
    expect(isMemoryCandidateRole("candidate")).toBe(true)
    expect(isMemoryCandidateRole("considered")).toBe(true)
    expect(isMemoryCandidateRole("selected")).toBe(false)
    expect(memoryReasonText({ kind: "claim", id: "c1", role: "candidate" }, t)).toBe(
      "It was considered as related context, but did not enter the answer context.",
    )
    expect(memoryReasonText({ kind: "claim", id: "c2", role: "considered" }, t)).toBe(
      "It was considered as related context, but did not enter the answer context.",
    )
    expect(
      memoryReasonText(
        { kind: "memory", id: "m1", origin: "active_memory", role: "considered" },
        t,
      ),
    ).toBe("Active recall considered it, but it was not the top pick.")
    expect(
      memoryReasonText({ kind: "claim", id: "c1", origin: "graph", role: "candidate" }, t),
    ).toBe(
      "Structured memory relationships expanded this into a candidate, but it was not automatically injected.",
    )
  })

  it("explains injected workflow guidance without calling it candidate-only", () => {
    expect(
      memoryReasonText(
        { kind: "procedure", id: "p1", origin: "experience", role: "injected" },
        t,
      ),
    ).toBe("A saved workflow entered this turn's soft guidance.")
    expect(
      memoryReasonText(
        { kind: "procedure", id: "p2", origin: "experience", role: "candidate" },
        t,
      ),
    ).toBe("A saved workflow matched this turn, but did not enter the answer context.")
    expect(
      memoryReasonText({ kind: "episode", id: "e1", origin: "experience", role: "candidate" }, t),
    ).toBe(
      "A related past experience was considered as context, but was not automatically injected.",
    )
  })

  it("formats retrieval trace top-level status and summary", () => {
    expect(retrievalTraceStatusLabel("candidates", t)).toBe("Related candidates")
    expect(retrievalTraceStatusLabel("partial", t)).toBe("Partially degraded")
    expect(retrievalTraceStatusLabel("degraded", t)).toBe("Degraded")
    expect(retrievalTraceStatusLabel("disabled", t)).toBe("Disabled")
    expect(retrievalTraceStatusLabel("no_context", t)).toBe("No matching context")

    expect(retrievalTraceTitle(0, { status: "degraded", totalRefs: 0, layers: [] }, t)).toBe(
      "Memory context",
    )
    expect(retrievalTraceTitle(2, { status: "candidates", totalRefs: 2, layers: [] }, t)).toBe(
      "Memory context",
    )
    expect(retrievalTraceTitle(1, { status: "partial", totalRefs: 1, layers: [] }, t)).toBe(
      "Used memory",
    )
    expect(retrievalTraceSummary(2, { status: "candidates", totalRefs: 2, layers: [] }, t)).toBe(
      "This turn found 2 related memory candidates; none were added to the answer context.",
    )
    expect(retrievalTraceSummary(2, { status: "partial", totalRefs: 2, layers: [] }, t)).toBe(
      "This answer used 2 long-term context items; some memory layers degraded.",
    )
    expect(retrievalTraceSummary(0, { status: "degraded", totalRefs: 0, layers: [] }, t)).toBe(
      "Memory retrieval degraded this turn, so no long-term context was added.",
    )
  })

  it("keeps trace-only panels quiet unless there is context or a degraded layer", () => {
    expect(shouldRenderMemoryTracePanel(1, undefined)).toBe(true)
    expect(
      shouldRenderMemoryTracePanel(0, {
        status: "degraded",
        totalRefs: 0,
        layers: [{ layer: "active_memory", status: "skipped", refCount: 0 }],
      }),
    ).toBe(true)
    expect(
      shouldRenderMemoryTracePanel(0, {
        status: "disabled",
        totalRefs: 0,
        layers: [
          {
            layer: "active_memory",
            status: "disabled",
            refCount: 0,
            skippedReason: "disabled",
          },
        ],
      }),
    ).toBe(false)
    expect(
      shouldRenderMemoryTracePanel(0, {
        status: "no_context",
        totalRefs: 0,
        layers: [
          {
            layer: "experience",
            status: "empty",
            refCount: 0,
            skippedReason: "no_candidates",
          },
        ],
      }),
    ).toBe(false)
  })
})
