import type { Message, RetrievalPlannerLayerTrace, UsedMemoryRef } from "@/types/chat"
import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const MAX_DEGRADED_ROWS = 6
const MAX_RECENT_TURNS = 8
const WORKSPACE_MEMORY_DIAGNOSTIC_MAX_CHARS = 420

export interface WorkspaceMemoryLayerSummary {
  layer: string
  turns: number
  used: number
  candidate: number
  empty: number
  skipped: number
  disabled: number
  refCount: number
  injectedCount: number
  selectedCount: number
  candidateCount: number
  droppedCount: number
  latencyMs: number | null
  cached: number
}

export interface WorkspaceMemoryDegradedLayer {
  layer: string
  status: string
  reason: string | null
  count: number
  latestLatencyMs: number | null
}

export interface WorkspaceMemoryTurnSummary {
  index: number
  status: string
  refCount: number
  contextRefCount: number
  candidateRefCount: number
  droppedCount: number
  degraded: boolean
  timestamp?: string
}

export interface WorkspaceMemoryDiagnostics {
  turns: number
  refCount: number
  contextRefCount: number
  candidateRefCount: number
  droppedCount: number
  statusCounts: Record<string, number>
  kindCounts: Record<string, number>
  originCounts: Record<string, number>
  layers: WorkspaceMemoryLayerSummary[]
  degradedLayers: WorkspaceMemoryDegradedLayer[]
  recentTurns: WorkspaceMemoryTurnSummary[]
  latest: {
    status: string
    refCount: number
    timestamp?: string
    index: number
  } | null
}

export type WorkspaceMemoryDiagnosticsTranslateFn = (
  key: string,
  defaultValue: string,
  options?: Record<string, unknown>,
) => string

export interface WorkspaceMemoryDiagnosticsErrorToast {
  title: string
  description?: string
}

export function buildWorkspaceMemoryDiagnostics(messages: Message[]): WorkspaceMemoryDiagnostics {
  const statusCounts = new Map<string, number>()
  const kindCounts = new Map<string, number>()
  const originCounts = new Map<string, number>()
  const layers = new Map<string, WorkspaceMemoryLayerSummary>()
  const degraded = new Map<string, WorkspaceMemoryDegradedLayer>()
  const turnSummaries: WorkspaceMemoryTurnSummary[] = []

  let turns = 0
  let refCount = 0
  let contextRefCount = 0
  let candidateRefCount = 0
  let droppedCount = 0
  let latest: WorkspaceMemoryDiagnostics["latest"] = null

  messages.forEach((message, index) => {
    if (message.role !== "assistant") return
    const refs = message.usedMemoryRefs ?? []
    const trace = message.retrievalPlanner
    if (refs.length === 0 && !trace?.layers.length) return

    turns += 1
    let turnContextRefCount = 0
    let turnCandidateRefCount = 0
    refCount += refs.length
    for (const ref of refs) {
      bump(kindCounts, ref.kind || "unknown")
      bump(originCounts, ref.origin || "unknown")
      if (isCandidateRef(ref)) {
        candidateRefCount += 1
        turnCandidateRefCount += 1
      } else {
        contextRefCount += 1
        turnContextRefCount += 1
      }
    }

    const status = trace?.status || "refs_only"
    bump(statusCounts, status)
    let turnDroppedCount = 0
    let degradedTurn = status === "partial" || status === "degraded"
    latest = {
      status,
      refCount: refs.length,
      timestamp: message.timestamp,
      index,
    }

    for (const layer of trace?.layers ?? []) {
      mergeLayer(layers, layer)
      const layerDroppedCount = safeNumber(layer.droppedCount)
      droppedCount += layerDroppedCount
      turnDroppedCount += layerDroppedCount
      if (layer.status === "skipped" || layer.status === "disabled") {
        degradedTurn = true
        mergeDegradedLayer(degraded, layer)
      }
    }

    turnSummaries.push({
      index,
      status,
      refCount: refs.length,
      contextRefCount: turnContextRefCount,
      candidateRefCount: turnCandidateRefCount,
      droppedCount: turnDroppedCount,
      degraded: degradedTurn,
      timestamp: message.timestamp,
    })
  })

  return {
    turns,
    refCount,
    contextRefCount,
    candidateRefCount,
    droppedCount,
    statusCounts: mapToObject(statusCounts),
    kindCounts: mapToObject(kindCounts),
    originCounts: mapToObject(originCounts),
    layers: [...layers.values()].sort(compareLayers),
    degradedLayers: [...degraded.values()].sort(compareDegraded).slice(0, MAX_DEGRADED_ROWS),
    recentTurns: turnSummaries.slice(-MAX_RECENT_TURNS),
    latest,
  }
}

export function workspaceMemoryDiagnosticsCopyErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? workspaceMemoryDiagnosticText(detail) : null
}

export function workspaceMemoryDiagnosticText(
  value: string,
  maxChars = WORKSPACE_MEMORY_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function workspaceMemoryDiagnosticsCopyErrorToast(
  t: WorkspaceMemoryDiagnosticsTranslateFn,
  error: unknown,
): WorkspaceMemoryDiagnosticsErrorToast {
  const detail = workspaceMemoryDiagnosticsCopyErrorDetail(error)
  const title = t(
    "workspace.memoryDiagnostics.copyFailed",
    "Failed to copy memory diagnostics",
  )
  if (!detail) return { title }
  return {
    title,
    description: t("workspace.memoryDiagnostics.copyDetail", "Details: {{error}}", {
      error: detail,
    }),
  }
}

export function formatWorkspaceMemoryDiagnosticsMarkdown(
  diagnostics: WorkspaceMemoryDiagnostics,
): string {
  const lines = [
    "# Workspace Memory Diagnostics",
    "",
    `- Turns with diagnostics: ${diagnostics.turns}`,
    `- Memory refs: ${diagnostics.refCount}`,
    `- Context refs: ${diagnostics.contextRefCount}`,
    `- Candidate refs: ${diagnostics.candidateRefCount}`,
    `- Dropped candidates: ${diagnostics.droppedCount}`,
  ]

  if (diagnostics.latest) {
    lines.push(
      `- Latest turn: #${diagnostics.latest.index + 1} · ${diagnostics.latest.status} · ${diagnostics.latest.refCount} refs`,
    )
  }

  appendCounts(lines, "Trace statuses", diagnostics.statusCounts)
  appendCounts(lines, "Memory kinds", diagnostics.kindCounts)
  appendCounts(lines, "Origins", diagnostics.originCounts)

  if (diagnostics.recentTurns.length > 0) {
    lines.push("", "## Recent Turns")
    for (const turn of diagnostics.recentTurns) {
      const degraded = turn.degraded ? " · degraded" : ""
      lines.push(
        `- #${turn.index + 1}: ${turn.status}, context=${turn.contextRefCount}, candidates=${turn.candidateRefCount}, dropped=${turn.droppedCount}${degraded}`,
      )
    }
  }

  if (diagnostics.layers.length > 0) {
    lines.push("", "## Layers")
    for (const layer of diagnostics.layers) {
      const latency = layer.latencyMs == null ? "" : ` · maxLatency=${layer.latencyMs}ms`
      lines.push(
        `- ${layer.layer}: turns=${layer.turns}, used=${layer.used}, candidate=${layer.candidate}, skipped=${layer.skipped}, disabled=${layer.disabled}, refs=${layer.refCount}, dropped=${layer.droppedCount}${latency}`,
      )
    }
  }

  if (diagnostics.degradedLayers.length > 0) {
    lines.push("", "## Degraded Layers")
    for (const layer of diagnostics.degradedLayers) {
      const reason = layer.reason ? ` · reason=${layer.reason}` : ""
      const latency = layer.latestLatencyMs == null ? "" : ` · latestLatency=${layer.latestLatencyMs}ms`
      lines.push(`- ${layer.layer}: ${layer.status} x${layer.count}${reason}${latency}`)
    }
  }

  return `${lines.join("\n")}\n`
}

function mergeLayer(
  layers: Map<string, WorkspaceMemoryLayerSummary>,
  layer: RetrievalPlannerLayerTrace,
) {
  const existing = layers.get(layer.layer) ?? {
    layer: layer.layer,
    turns: 0,
    used: 0,
    candidate: 0,
    empty: 0,
    skipped: 0,
    disabled: 0,
    refCount: 0,
    injectedCount: 0,
    selectedCount: 0,
    candidateCount: 0,
    droppedCount: 0,
    latencyMs: null,
    cached: 0,
  }
  existing.turns += 1
  if (layer.status === "used") existing.used += 1
  else if (layer.status === "candidate") existing.candidate += 1
  else if (layer.status === "empty") existing.empty += 1
  else if (layer.status === "skipped") existing.skipped += 1
  else if (layer.status === "disabled") existing.disabled += 1
  existing.refCount += safeNumber(layer.refCount)
  existing.injectedCount += safeNumber(layer.injectedCount)
  existing.selectedCount += safeNumber(layer.selectedCount)
  existing.candidateCount += safeNumber(layer.candidateCount)
  existing.droppedCount += safeNumber(layer.droppedCount)
  existing.latencyMs = maxNullable(existing.latencyMs, layer.latencyMs)
  if (layer.cached) existing.cached += 1
  layers.set(layer.layer, existing)
}

function mergeDegradedLayer(
  degraded: Map<string, WorkspaceMemoryDegradedLayer>,
  layer: RetrievalPlannerLayerTrace,
) {
  const reason = layer.skippedReason ?? null
  const key = `${layer.layer}\0${layer.status}\0${reason ?? ""}`
  const existing = degraded.get(key) ?? {
    layer: layer.layer,
    status: layer.status,
    reason,
    count: 0,
    latestLatencyMs: null,
  }
  existing.count += 1
  existing.latestLatencyMs = layer.latencyMs ?? null
  degraded.set(key, existing)
}

function appendCounts(lines: string[], title: string, counts: Record<string, number>) {
  const entries = Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
  if (entries.length === 0) return
  lines.push("", `## ${title}`)
  for (const [key, count] of entries) {
    lines.push(`- ${key}: ${count}`)
  }
}

function isCandidateRef(ref: UsedMemoryRef): boolean {
  return ref.role === "candidate" || ref.role === "considered"
}

function safeNumber(value: number | null | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

function maxNullable(a: number | null, b: number | null | undefined): number | null {
  if (typeof b !== "number" || !Number.isFinite(b)) return a
  return a == null ? b : Math.max(a, b)
}

function bump(map: Map<string, number>, key: string) {
  map.set(key, (map.get(key) ?? 0) + 1)
}

function mapToObject(map: Map<string, number>): Record<string, number> {
  return Object.fromEntries([...map.entries()].sort((a, b) => a[0].localeCompare(b[0])))
}

function compareLayers(a: WorkspaceMemoryLayerSummary, b: WorkspaceMemoryLayerSummary): number {
  const degradedA = a.skipped + a.disabled
  const degradedB = b.skipped + b.disabled
  return degradedB - degradedA || b.refCount - a.refCount || a.layer.localeCompare(b.layer)
}

function compareDegraded(
  a: WorkspaceMemoryDegradedLayer,
  b: WorkspaceMemoryDegradedLayer,
): number {
  return b.count - a.count || a.layer.localeCompare(b.layer) || a.status.localeCompare(b.status)
}
