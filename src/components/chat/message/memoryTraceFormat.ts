import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"
import type { RetrievalPlannerTrace, UsedMemoryRef } from "@/types/chat"

const MEMORY_TRACE_DIAGNOSTIC_MAX_CHARS = 420

export interface TranslateFn {
  (key: string, defaultValue: string): string
  (key: string, options: Record<string, unknown>): string
}

export function retrievalLayerStatusLabel(status: string, t: TranslateFn): string {
  switch (status) {
    case "used":
      return t("chat.memoryTrace.layerStatus.used", "used")
    case "candidate":
      return t("chat.memoryTrace.layerStatus.candidate", "candidate")
    case "empty":
      return t("chat.memoryTrace.layerStatus.empty", "no hit")
    case "skipped":
      return t("chat.memoryTrace.layerStatus.skipped", "skipped")
    case "disabled":
      return t("chat.memoryTrace.layerStatus.disabled", "disabled")
    default:
      return status || t("chat.memoryTrace.layerStatus.unknown", "unknown")
  }
}

export function retrievalTraceStatusLabel(status: string, t: TranslateFn): string {
  switch (status) {
    case "used":
      return t("chat.memoryTrace.traceStatus.used", "Used")
    case "candidates":
      return t("chat.memoryTrace.traceStatus.candidates", "Related candidates")
    case "partial":
      return t("chat.memoryTrace.traceStatus.partial", "Partially degraded")
    case "degraded":
      return t("chat.memoryTrace.traceStatus.degraded", "Degraded")
    case "disabled":
      return t("chat.memoryTrace.traceStatus.disabled", "Disabled")
    case "no_context":
      return t("chat.memoryTrace.traceStatus.noContext", "No matching context")
    default:
      return status || t("chat.memoryTrace.traceStatus.unknown", "Unknown")
  }
}

export function retrievalIntentLabel(intent: string, t: TranslateFn): string {
  switch (intent) {
    case "profile":
      return t("chat.memoryTrace.intent.profile", "Profile")
    case "procedure":
      return t("chat.memoryTrace.intent.procedure", "Workflow")
    case "episode":
      return t("chat.memoryTrace.intent.episode", "Past experience")
    case "relationship":
      return t("chat.memoryTrace.intent.relationship", "Relationships")
    case "knowledge":
      return t("chat.memoryTrace.intent.knowledge", "Knowledge")
    case "general":
      return t("chat.memoryTrace.intent.general", "General")
    default:
      return intent
  }
}

export function memoryKindLabel(ref: Pick<UsedMemoryRef, "kind">, t: TranslateFn): string {
  switch (ref.kind) {
    case "memory":
      return t("chat.memoryTrace.kind.memory", "Memory")
    case "claim":
      return t("chat.memoryTrace.kind.claim", "Structured memory")
    case "profile":
      return t("chat.memoryTrace.kind.profile", "Profile")
    case "knowledge":
      return t("chat.memoryTrace.kind.knowledge", "Knowledge note")
    case "episode":
      return t("chat.memoryTrace.kind.episode", "Experience")
    case "procedure":
      return t("chat.memoryTrace.kind.procedure", "Workflow")
    default:
      return ref.kind
  }
}

export function memoryOriginLabel(origin: UsedMemoryRef["origin"], t: TranslateFn): string {
  switch (origin) {
    case "active_memory":
      return t("chat.memoryTrace.origin.activeMemory", "Active recall")
    case "pinned_memory":
      return t("chat.memoryTrace.origin.pinnedMemory", "Pinned memory")
    case "static_memory":
      return t("chat.memoryTrace.origin.staticMemory", "Long-term memory")
    case "profile":
      return t("chat.memoryTrace.origin.profile", "Profile")
    case "knowledge":
      return t("chat.memoryTrace.origin.knowledge", "Knowledge note")
    case "experience":
      return t("chat.memoryTrace.origin.experience", "Experience memory")
    case "graph":
      return t("chat.memoryTrace.origin.graph", "Entity relationships")
    case "project_auto_memory":
      return t("project.tabAutoMemory", "Auto Memory")
    case "core_memory":
      return t("settings.memoryV2.core.title", "Always remembered")
    default:
      return origin || t("chat.memoryTrace.origin.unknown", "Long-term context")
  }
}

export function memoryScopeLabel(scope: UsedMemoryRef["scope"], t: TranslateFn): string | null {
  const value = scope?.trim()
  if (!value) return null
  const normalized = value.toLowerCase()
  if (normalized === "global") {
    return t("settings.memoryScopeGlobal", "Global")
  }
  if (normalized === "project") {
    return t("settings.memoryScopeProject", "Project")
  }
  if (normalized.startsWith("project:")) {
    const id = value.slice(value.indexOf(":") + 1).trim()
    const label = t("settings.memoryScopeProject", "Project")
    return id ? `${label}: ${id}` : label
  }
  if (normalized === "agent") {
    return t("settings.memoryScopeAgent", "Agent")
  }
  if (normalized.startsWith("agent:")) {
    const id = value.slice(value.indexOf(":") + 1).trim()
    const label = t("settings.memoryScopeAgent", "Agent")
    return id ? `${label}: ${id}` : label
  }
  if (normalized === "session") {
    return t("dashboard.columns.session", "Session")
  }
  if (normalized.startsWith("session:")) {
    const id = value.slice(value.indexOf(":") + 1).trim()
    const label = t("dashboard.columns.session", "Session")
    return id ? `${label}: ${id}` : label
  }
  return value
}

export function memorySourceLabel(
  ref: Pick<UsedMemoryRef, "sourceType" | "scope">,
  t: TranslateFn,
): string {
  const sourceType =
    ref.sourceType === "project_auto_memory_index"
      ? t("project.tabAutoMemory", "Auto Memory")
      : ref.sourceType === "core_memory_index"
        ? "MEMORY.md"
        : ref.sourceType
  return [sourceType, memoryScopeLabel(ref.scope, t)].filter(Boolean).join(" · ")
}

export function retrievalLayerLabel(layer: string, t: TranslateFn): string {
  switch (layer) {
    case "context_pack":
      return t("chat.memoryTrace.layer.contextPack", "Pinned memory")
    case "static_memory":
      return t("chat.memoryTrace.layer.staticMemory", "Long-term memory")
    case "profile":
      return t("chat.memoryTrace.layer.profile", "Profile")
    case "active_memory":
      return t("chat.memoryTrace.layer.activeMemory", "Active recall")
    case "knowledge":
      return t("chat.memoryTrace.layer.knowledge", "Knowledge note")
    case "experience":
      return t("chat.memoryTrace.layer.experience", "Experience memory")
    case "graph":
      return t("chat.memoryTrace.layer.graph", "Entity relationships")
    default:
      return layer || t("chat.memoryTrace.layer.unknown", "Context layer")
  }
}

export function memoryRoleLabel(role: UsedMemoryRef["role"], t: TranslateFn): string | null {
  if (isMemoryCandidateRole(role)) {
    return t("chat.memoryTrace.role.candidate", "Candidate")
  }
  switch (role) {
    case "selected":
      return t("chat.memoryTrace.role.selected", "Selected")
    case "injected":
      return t("chat.memoryTrace.role.injected", "Injected")
    default:
      return role ?? null
  }
}

export function isMemoryCandidateRole(role: UsedMemoryRef["role"]): boolean {
  return role === "candidate" || role === "considered"
}

export function retrievalTraceTitle(
  refCount: number,
  trace: RetrievalPlannerTrace | undefined,
  t: TranslateFn,
): string {
  if (!trace && refCount > 0) {
    return t("chat.memoryTrace.title", "Used memory")
  }
  if (trace?.status === "used" || trace?.status === "partial") {
    return t("chat.memoryTrace.title", "Used memory")
  }
  return t("chat.memoryTrace.contextTitle", "Memory context")
}

export function retrievalTraceSummary(
  refCount: number,
  trace: RetrievalPlannerTrace | undefined,
  t: TranslateFn,
): string {
  switch (trace?.status) {
    case "candidates":
      return t("chat.memoryTrace.summaryCandidates", {
        count: refCount,
        defaultValue:
          "This turn found {{count}} related memory candidates; none were added to the answer context.",
      })
    case "partial":
      return t("chat.memoryTrace.summaryPartial", {
        count: refCount,
        defaultValue:
          "This answer used {{count}} long-term context items; some memory layers degraded.",
      })
    case "degraded":
      return t("chat.memoryTrace.summaryDegraded", {
        defaultValue: "Memory retrieval degraded this turn, so no long-term context was added.",
      })
    case "disabled":
      return t("chat.memoryTrace.summaryDisabled", {
        defaultValue: "Memory retrieval was disabled for this turn.",
      })
    case "no_context":
      return t("chat.memoryTrace.summaryNoContext", {
        defaultValue: "No matching long-term context was found for this turn.",
      })
    default:
      return t("chat.memoryTrace.summary", {
        count: refCount,
        defaultValue: "This answer used {{count}} long-term context items.",
      })
  }
}

export function shouldRenderMemoryTracePanel(
  refCount: number,
  trace: RetrievalPlannerTrace | undefined,
): boolean {
  if (refCount > 0) return true
  if (!trace?.layers.length) return false
  return trace.status === "partial" || trace.status === "degraded"
}

export function retrievalLayerReasonLabel(
  reason: string | null | undefined,
  t: TranslateFn,
): string | null {
  if (!reason) return null
  switch (reason) {
    case "incognito":
      return t("chat.memoryTrace.layerReason.incognito", "incognito session")
    case "disabled":
      return t("chat.memoryTrace.layerReason.disabled", "configuration off")
    case "memory_off":
    case "agent_memory_off":
      return t("chat.memoryTrace.layerReason.memoryOff", "long-term memory is off")
    case "agent_config_error":
    case "agent_config_unavailable":
      return t("chat.memoryTrace.layerReason.agentConfigError", "agent memory config unavailable")
    case "empty_query":
      return t("chat.memoryTrace.layerReason.emptyQuery", "empty query")
    case "no_candidates":
      return t("chat.memoryTrace.layerReason.noCandidates", "no candidates")
    case "non_contextual_turn":
      return t("chat.memoryTrace.layerReason.nonContextualTurn", "non-contextual turn")
    case "recall_off":
      return t("chat.memoryTrace.layerReason.recallOff", "automatic dynamic recall is off")
    case "budget_empty":
      return t("chat.memoryTrace.layerReason.budgetEmpty", "recall budget is empty")
    case "no_graph_neighbors":
      return t("chat.memoryTrace.layerReason.noGraphNeighbors", "no related entity relationships")
    case "no_access":
      return t("chat.memoryTrace.layerReason.noAccess", "no accessible knowledge base")
    case "no_hits":
      return t("chat.memoryTrace.layerReason.noHits", "no hits")
    case "llm_none":
    case "deep_none":
      return t("chat.memoryTrace.layerReason.llmNone", "model selected none")
    case "timeout":
    case "retrieval_timeout":
      return t("chat.memoryTrace.layerReason.timeout", "timed out")
    case "retrieval_busy":
      return t("chat.memoryTrace.layerReason.retrievalBusy", "retrieval is busy")
    case "side_query_error":
      return t("chat.memoryTrace.layerReason.sideQueryError", "recall failed")
    case "retrieval_error":
      return t("chat.memoryTrace.layerReason.retrievalError", "retrieval failed")
    case "no_session":
      return t("chat.memoryTrace.layerReason.noSession", "no session context")
    case "session_policy":
    case "memory_off_or_session_policy":
      return t("chat.memoryTrace.layerReason.sessionPolicy", "disabled by memory or chat policy")
    case "unified_dynamic_recall":
      return t("chat.memoryTrace.layerReason.unifiedRecall", "handled by fast recall")
    default:
      return t("chat.memoryTrace.layerReason.unknown", "other reason")
  }
}

export function retrievalLayerDetailParts(
  layer: RetrievalPlannerTrace["layers"][number],
  t: TranslateFn,
): string[] {
  return [
    retrievalLayerStatusLabel(layer.status, t),
    layer.refCount > 0 ? `refs=${layer.refCount}` : null,
    typeof layer.injectedCount === "number" ? `injected=${layer.injectedCount}` : null,
    typeof layer.selectedCount === "number" ? `selected=${layer.selectedCount}` : null,
    typeof layer.candidateCount === "number" ? `candidates=${layer.candidateCount}` : null,
    typeof layer.droppedCount === "number" && layer.droppedCount > 0
      ? `dropped=${layer.droppedCount}`
      : null,
    typeof layer.latencyMs === "number" ? `${layer.latencyMs}ms` : null,
    layer.cached ? t("chat.memoryTrace.cached", "cached") : null,
    retrievalLayerReasonLabel(layer.skippedReason, t),
  ].filter((part): part is string => !!part)
}

export function formatMemoryMetric(value: number | undefined): string | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null
  if (value >= 0 && value <= 1) return `${Math.round(value * 100)}%`
  return value.toFixed(2)
}

export function memoryMetricLabels(ref: UsedMemoryRef, t: TranslateFn): string[] {
  const labels: string[] = []
  const score = formatMemoryMetric(ref.score)
  if (score) labels.push(t("chat.memoryTrace.metric.score", { value: score }))
  const confidence = formatMemoryMetric(ref.confidence)
  if (confidence) labels.push(t("chat.memoryTrace.metric.confidence", { value: confidence }))
  const salience = formatMemoryMetric(ref.salience)
  if (salience) labels.push(t("chat.memoryTrace.metric.salience", { value: salience }))
  return labels
}

export function memoryTraceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return memoryTraceDiagnosticText(detail)
}

export function memoryTraceDiagnosticText(
  value: string,
  maxChars = MEMORY_TRACE_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function memoryTraceErrorDescription(error: unknown, t: TranslateFn): string | undefined {
  const detail = memoryTraceErrorDetail(error)
  if (!detail) return undefined
  return t("chat.memoryTrace.errorDetail", {
    defaultValue: "Details: {{error}}",
    error: detail,
  })
}

export function memoryReasonText(ref: UsedMemoryRef, t: TranslateFn): string {
  if (ref.origin === "active_memory" && ref.role === "selected") {
    return t(
      "chat.memoryTrace.reason.activeSelected",
      "Active recall selected it for the current question.",
    )
  }
  if (ref.origin === "active_memory" && isMemoryCandidateRole(ref.role)) {
    return t(
      "chat.memoryTrace.reason.activeCandidate",
      "Active recall considered it, but it was not the top pick.",
    )
  }
  if (ref.origin === "pinned_memory") {
    return t(
      "chat.memoryTrace.reason.pinned",
      "A high-salience pinned memory entered this turn's context.",
    )
  }
  if (ref.origin === "static_memory") {
    return t(
      "chat.memoryTrace.reason.static",
      "It fit within the long-term memory budget for this turn.",
    )
  }
  if (ref.origin === "profile") {
    return t(
      "chat.memoryTrace.reason.profile",
      "The user profile summary entered this turn's context.",
    )
  }
  if (ref.origin === "knowledge") {
    return t(
      "chat.memoryTrace.reason.knowledge",
      "A related knowledge note entered this turn's context.",
    )
  }
  if (ref.origin === "experience" && ref.kind === "procedure" && ref.role === "injected") {
    return t(
      "chat.memoryTrace.reason.procedureInjected",
      "A saved workflow entered this turn's soft guidance.",
    )
  }
  if (ref.origin === "experience" && ref.kind === "procedure") {
    return t(
      "chat.memoryTrace.reason.procedureCandidate",
      "A saved workflow matched this turn, but did not enter the answer context.",
    )
  }
  if (ref.origin === "experience" && ref.kind === "episode") {
    return t(
      "chat.memoryTrace.reason.episodeCandidate",
      "A related past experience was considered as context, but was not automatically injected.",
    )
  }
  if (ref.origin === "experience") {
    return t(
      "chat.memoryTrace.reason.experience",
      "A related experience memory was considered, but was not automatically injected.",
    )
  }
  if (ref.origin === "graph") {
    return t(
      "chat.memoryTrace.reason.graph",
      "Structured memory relationships expanded this into a candidate, but it was not automatically injected.",
    )
  }
  if (ref.role === "injected") {
    return t("chat.memoryTrace.reason.injected", "It entered this turn's context.")
  }
  if (isMemoryCandidateRole(ref.role)) {
    return t(
      "chat.memoryTrace.reason.candidate",
      "It was considered as related context, but did not enter the answer context.",
    )
  }
  return t("chat.memoryTrace.reason.related", "It is related to this answer.")
}

export function memoryLocationLabel(ref: UsedMemoryRef): string | null {
  const parts: string[] = []
  const heading = ref.headingPath?.trim()
  if (heading) parts.push(heading)
  const blockId = ref.blockId?.trim()
  if (blockId) parts.push(blockId.startsWith("^") ? blockId : `^${blockId}`)
  const line =
    typeof ref.line === "number" && Number.isFinite(ref.line) && ref.line > 0
      ? Math.floor(ref.line)
      : null
  const col =
    typeof ref.col === "number" && Number.isFinite(ref.col) && ref.col > 0
      ? Math.floor(ref.col)
      : null
  if (line && col) parts.push(`L${line}:C${col}`)
  else if (line) parts.push(`L${line}`)
  else if (col) parts.push(`C${col}`)
  return parts.length > 0 ? parts.join(" · ") : null
}
