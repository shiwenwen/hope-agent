export const MEMORY_FOCUS_EVENT = "hope:memory-focus"

const STORAGE_KEY = "hope.memory.focus"
const HASH_ROOT = "memory"

export interface ClaimFocusFilters {
  statusFilter?: string
  claimType?: string | null
  scopeType?: string | null
  scopeId?: string | null
  confidenceSource?: string | null
  evidenceClass?: string | null
  evidenceSource?: string | null
  claimSort?: string | null
  claimLoaded?: number | null
  query?: string | null
  reviewHistory?: boolean
  reviewHistoryDecisionType?: string | null
  reviewHistoryTimeRange?: string | null
  reviewHistoryScopeType?: string | null
  reviewHistoryScopeId?: string | null
  reviewHistoryQuery?: string | null
}

export interface ClaimFocusState extends ClaimFocusFilters {
  nonce: number
  selectedId?: string | null
}

export interface MemoryOverviewFocus {
  auditOpen?: boolean
  auditAction?: string | null
  auditQuery?: string | null
}

export type MemoryFocusTarget =
  | ({ kind: "overview" } & MemoryOverviewFocus)
  | { kind: "memory"; id: number }
  | ({ kind: "claims"; selectedId?: string | null } & ClaimFocusFilters)
  | ({ kind: "claim"; id: string } & ClaimFocusFilters)
  | { kind: "episode"; id: string }
  | { kind: "procedure"; id: string }
  | { kind: "profile"; id?: string }

interface RequestMemoryFocusOptions {
  updateUrl?: boolean
  replace?: boolean
}

function normalizeTarget(value: unknown): MemoryFocusTarget | null {
  if (!value || typeof value !== "object") return null
  const raw = value as Record<string, unknown>
  if (raw.kind === "overview") {
    return { kind: "overview", ...normalizeOverviewFocusFields(raw) }
  }
  if (raw.kind === "memory") {
    const id = typeof raw.id === "number" ? raw.id : Number(raw.id)
    return Number.isFinite(id) ? { kind: "memory", id } : null
  }
  if (raw.kind === "claims") {
    return { kind: "claims", ...normalizeClaimFocusFields(raw) }
  }
  if (raw.kind === "claim" && typeof raw.id === "string" && raw.id.length > 0) {
    return { kind: "claim", id: raw.id, ...normalizeClaimFocusFields(raw) }
  }
  if (
    (raw.kind === "episode" || raw.kind === "procedure") &&
    typeof raw.id === "string" &&
    raw.id.length > 0
  ) {
    return { kind: raw.kind, id: raw.id }
  }
  if (raw.kind === "profile") {
    return typeof raw.id === "string" ? { kind: "profile", id: raw.id } : { kind: "profile" }
  }
  return null
}

const MEMORY_AUDIT_ACTIONS = new Set(["all", "add", "update", "delete", "pin", "unpin", "import"])

function cleanString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined
  const trimmed = value.trim()
  return trimmed ? trimmed : undefined
}

function cleanNullableString(value: unknown): string | null | undefined {
  if (value === null) return null
  return cleanString(value)
}

function cleanPositiveInt(value: unknown): number | undefined {
  const number = typeof value === "number" ? value : typeof value === "string" ? Number(value) : NaN
  if (!Number.isFinite(number) || number <= 0) return undefined
  return Math.floor(number)
}

function cleanMemoryAuditAction(value: unknown): string | undefined {
  const action = cleanString(value)
  return action && MEMORY_AUDIT_ACTIONS.has(action) ? action : undefined
}

export function buildClaimFocusState(
  focus: (ClaimFocusFilters & { selectedId?: string | null }) | undefined,
  previousNonce = 0,
): ClaimFocusState {
  return {
    nonce: previousNonce + 1,
    ...focus,
  }
}

function normalizeOverviewFocusFields(raw: Record<string, unknown>): MemoryOverviewFocus {
  const fields: MemoryOverviewFocus = {}
  if (typeof raw.auditOpen === "boolean") fields.auditOpen = raw.auditOpen
  const auditAction = cleanMemoryAuditAction(raw.auditAction)
  if (auditAction !== undefined) fields.auditAction = auditAction
  const auditQuery = cleanNullableString(raw.auditQuery)
  if (auditQuery !== undefined) fields.auditQuery = auditQuery
  return fields
}

function normalizeClaimFocusFields(raw: Record<string, unknown>): ClaimFocusFilters & {
  selectedId?: string | null
} {
  const fields: ClaimFocusFilters & { selectedId?: string | null } = {}
  const selectedId = cleanNullableString(raw.selectedId)
  if (selectedId !== undefined) fields.selectedId = selectedId
  const statusFilter = cleanString(raw.statusFilter)
  if (statusFilter) fields.statusFilter = statusFilter
  const claimType = cleanNullableString(raw.claimType)
  if (claimType !== undefined) fields.claimType = claimType
  const scopeType = cleanNullableString(raw.scopeType)
  if (scopeType !== undefined) fields.scopeType = scopeType
  const scopeId = cleanNullableString(raw.scopeId)
  if (scopeId !== undefined) fields.scopeId = scopeId
  const confidenceSource = cleanNullableString(raw.confidenceSource)
  if (confidenceSource !== undefined) fields.confidenceSource = confidenceSource
  const evidenceClass = cleanNullableString(raw.evidenceClass)
  if (evidenceClass !== undefined) fields.evidenceClass = evidenceClass
  const evidenceSource = cleanNullableString(raw.evidenceSource)
  if (evidenceSource !== undefined) fields.evidenceSource = evidenceSource
  const claimSort = cleanNullableString(raw.claimSort)
  if (claimSort !== undefined) fields.claimSort = claimSort
  const claimLoaded = cleanPositiveInt(raw.claimLoaded)
  if (claimLoaded !== undefined) fields.claimLoaded = claimLoaded
  const query = cleanNullableString(raw.query)
  if (query !== undefined) fields.query = query
  if (typeof raw.reviewHistory === "boolean") fields.reviewHistory = raw.reviewHistory
  const reviewHistoryDecisionType = cleanNullableString(raw.reviewHistoryDecisionType)
  if (reviewHistoryDecisionType !== undefined) {
    fields.reviewHistoryDecisionType = reviewHistoryDecisionType
  }
  const reviewHistoryTimeRange = cleanNullableString(raw.reviewHistoryTimeRange)
  if (reviewHistoryTimeRange !== undefined) fields.reviewHistoryTimeRange = reviewHistoryTimeRange
  const reviewHistoryScopeType = cleanNullableString(raw.reviewHistoryScopeType)
  if (reviewHistoryScopeType !== undefined) fields.reviewHistoryScopeType = reviewHistoryScopeType
  const reviewHistoryScopeId = cleanNullableString(raw.reviewHistoryScopeId)
  if (reviewHistoryScopeId !== undefined) fields.reviewHistoryScopeId = reviewHistoryScopeId
  const reviewHistoryQuery = cleanNullableString(raw.reviewHistoryQuery)
  if (reviewHistoryQuery !== undefined) fields.reviewHistoryQuery = reviewHistoryQuery
  return fields
}

function appendOverviewFocusParams(params: URLSearchParams, target: MemoryOverviewFocus): void {
  const auditAction = cleanMemoryAuditAction(target.auditAction) ?? "all"
  const auditQuery = target.auditQuery?.trim()
  if (target.auditOpen || auditAction !== "all" || auditQuery) {
    params.set("audit", "1")
  }
  if (auditAction !== "all") params.set("auditAction", auditAction)
  if (auditQuery) params.set("auditQ", auditQuery)
}

function appendClaimFocusParams(
  params: URLSearchParams,
  target: ClaimFocusFilters & { selectedId?: string | null },
): void {
  if (target.statusFilter && target.statusFilter !== "all") {
    params.set("status", target.statusFilter)
  }
  if (target.claimType && target.claimType !== "all") params.set("type", target.claimType)
  if (target.scopeType === "global") {
    params.set("scope", "global")
  } else if ((target.scopeType === "agent" || target.scopeType === "project") && target.scopeId) {
    params.set("scope", `${target.scopeType}:${target.scopeId}`)
  }
  if (target.confidenceSource && target.confidenceSource !== "all") {
    params.set("confidence", target.confidenceSource)
  }
  if (target.evidenceClass && target.evidenceClass !== "all") {
    params.set("evidenceClass", target.evidenceClass)
  }
  if (target.evidenceSource && target.evidenceSource !== "all") {
    params.set("source", target.evidenceSource)
  }
  if (target.claimSort && target.claimSort !== "relevance") {
    params.set("sort", target.claimSort)
  }
  if (typeof target.claimLoaded === "number" && target.claimLoaded > 0) {
    params.set("loaded", String(Math.floor(target.claimLoaded)))
  }
  if (target.query) params.set("q", target.query)
  if (target.reviewHistory) {
    params.set("status", "needs_review")
    params.set("history", "1")
    if (target.reviewHistoryDecisionType && target.reviewHistoryDecisionType !== "all") {
      params.set("historyDecision", target.reviewHistoryDecisionType)
    }
    if (target.reviewHistoryTimeRange && target.reviewHistoryTimeRange !== "all") {
      params.set("historyRange", target.reviewHistoryTimeRange)
    }
    if (target.reviewHistoryScopeType === "global") {
      params.set("historyScope", "global")
    } else if (
      (target.reviewHistoryScopeType === "agent" || target.reviewHistoryScopeType === "project") &&
      target.reviewHistoryScopeId
    ) {
      params.set("historyScope", `${target.reviewHistoryScopeType}:${target.reviewHistoryScopeId}`)
    }
    if (target.reviewHistoryQuery) params.set("historyQ", target.reviewHistoryQuery)
  }
  if (target.selectedId) params.set("selected", target.selectedId)
}

function parseScopeParam(value: string | null): Pick<ClaimFocusFilters, "scopeType" | "scopeId"> {
  if (!value) return {}
  if (value === "global") return { scopeType: "global", scopeId: null }
  const separator = value.indexOf(":")
  if (separator <= 0) return {}
  const scopeType = value.slice(0, separator)
  const scopeId = value.slice(separator + 1)
  if ((scopeType === "agent" || scopeType === "project") && scopeId) return { scopeType, scopeId }
  return {}
}

function parseHistoryScopeParam(
  value: string | null,
): Pick<ClaimFocusFilters, "reviewHistoryScopeType" | "reviewHistoryScopeId"> {
  if (!value) return {}
  if (value === "global") return { reviewHistoryScopeType: "global", reviewHistoryScopeId: null }
  const separator = value.indexOf(":")
  if (separator <= 0) return {}
  const scopeType = value.slice(0, separator)
  const scopeId = value.slice(separator + 1)
  if ((scopeType === "agent" || scopeType === "project") && scopeId) {
    return { reviewHistoryScopeType: scopeType, reviewHistoryScopeId: scopeId }
  }
  return {}
}

function parseClaimFocusParams(query: string): ClaimFocusFilters & { selectedId?: string | null } {
  const params = new URLSearchParams(query)
  const fields: ClaimFocusFilters & { selectedId?: string | null } = {
    ...parseScopeParam(params.get("scope")),
    ...parseHistoryScopeParam(params.get("historyScope")),
  }
  const selected = params.get("selected")
  if (selected) fields.selectedId = selected
  const status = params.get("status")
  if (status) fields.statusFilter = status
  const type = params.get("type")
  if (type) fields.claimType = type
  const confidence = params.get("confidence")
  if (confidence) fields.confidenceSource = confidence
  const evidenceClass = params.get("evidenceClass")
  if (evidenceClass) fields.evidenceClass = evidenceClass
  const source = params.get("source")
  if (source) fields.evidenceSource = source
  const sort = params.get("sort")
  if (sort) fields.claimSort = sort
  const loaded = cleanPositiveInt(params.get("loaded"))
  if (loaded !== undefined) fields.claimLoaded = loaded
  const q = params.get("q")
  if (q) fields.query = q
  if (params.get("history") === "1") {
    fields.reviewHistory = true
    fields.statusFilter = "needs_review"
  }
  const historyDecision = params.get("historyDecision")
  if (historyDecision) fields.reviewHistoryDecisionType = historyDecision
  const historyRange = params.get("historyRange")
  if (historyRange) fields.reviewHistoryTimeRange = historyRange
  const historyQ = params.get("historyQ")
  if (historyQ) fields.reviewHistoryQuery = historyQ
  return fields
}

function parseOverviewFocusParams(query: string): MemoryOverviewFocus {
  const params = new URLSearchParams(query)
  const fields: MemoryOverviewFocus = {}
  if (params.get("audit") === "1") fields.auditOpen = true
  const auditAction = cleanMemoryAuditAction(params.get("auditAction"))
  if (auditAction !== undefined) {
    fields.auditAction = auditAction
    fields.auditOpen = true
  }
  const auditQuery = cleanNullableString(params.get("auditQ"))
  if (auditQuery !== undefined) {
    fields.auditQuery = auditQuery
    if (auditQuery) fields.auditOpen = true
  }
  return fields
}

function targetHashPath(target: MemoryFocusTarget): string {
  if (target.kind === "overview") {
    const params = new URLSearchParams()
    appendOverviewFocusParams(params, target)
    const query = params.toString()
    return query ? `${HASH_ROOT}/overview?${query}` : `${HASH_ROOT}/overview`
  }
  if (target.kind === "memory") return `${HASH_ROOT}/memory/${target.id}`
  if (target.kind === "episode" || target.kind === "procedure") {
    return `${HASH_ROOT}/${target.kind}/${encodeURIComponent(target.id)}`
  }
  if (target.kind === "claims" || target.kind === "claim") {
    const path =
      target.kind === "claim"
        ? `${HASH_ROOT}/claim/${encodeURIComponent(target.id)}`
        : `${HASH_ROOT}/claims`
    const params = new URLSearchParams()
    appendClaimFocusParams(params, target)
    const query = params.toString()
    return query ? `${path}?${query}` : path
  }
  if (target.id) return `${HASH_ROOT}/profile/${encodeURIComponent(target.id)}`
  return `${HASH_ROOT}/profile`
}

export function memoryFocusHash(target: MemoryFocusTarget): string {
  return `#${targetHashPath(target)}`
}

export function memoryFocusUrl(target: MemoryFocusTarget): string {
  if (typeof window === "undefined") return memoryFocusHash(target)
  const url = new URL(window.location.href)
  url.hash = targetHashPath(target)
  return url.toString()
}

export function parseMemoryFocusHash(hash: string): MemoryFocusTarget | null {
  const fragment = hash.startsWith("#") ? hash.slice(1) : hash
  const queryStart = fragment.indexOf("?")
  const path = queryStart >= 0 ? fragment.slice(0, queryStart) : fragment
  const query = queryStart >= 0 ? fragment.slice(queryStart + 1) : ""
  const [root, kind, rawId] = path.split("/")
  if (root !== HASH_ROOT) return null
  try {
    if (kind === "overview") {
      return { kind: "overview", ...parseOverviewFocusParams(query) }
    }
    if (kind === "memory") {
      const id = Number(rawId)
      return Number.isFinite(id) ? { kind: "memory", id } : null
    }
    if (kind === "claims") {
      return { kind: "claims", ...parseClaimFocusParams(query) }
    }
    if (kind === "claim" && rawId) {
      const id = decodeURIComponent(rawId)
      return id ? { kind: "claim", id, ...parseClaimFocusParams(query) } : null
    }
    if ((kind === "episode" || kind === "procedure") && rawId) {
      const id = decodeURIComponent(rawId)
      return id ? { kind, id } : null
    }
    if (kind === "profile") {
      const id = rawId ? decodeURIComponent(rawId) : ""
      return id ? { kind: "profile", id } : { kind: "profile" }
    }
  } catch {
    return null
  }
  return null
}

export function parseMemoryFocusFromLocation(): MemoryFocusTarget | null {
  if (typeof window === "undefined") return null
  return parseMemoryFocusHash(window.location.hash)
}

function updateMemoryFocusUrl(target: MemoryFocusTarget, replace: boolean): void {
  if (typeof window === "undefined") return
  const url = memoryFocusUrl(target)
  if (replace) window.history.replaceState(window.history.state, "", url)
  else window.history.pushState(window.history.state, "", url)
}

export function setMemoryFocusUrl(target: MemoryFocusTarget, replace = true): void {
  updateMemoryFocusUrl(target, replace)
}

export function requestMemoryFocus(
  target: MemoryFocusTarget,
  options: RequestMemoryFocusOptions = {},
): void {
  if (typeof window === "undefined") return
  const { updateUrl = true, replace = true } = options
  if (updateUrl) updateMemoryFocusUrl(target, replace)
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(target))
  } catch {
    /* sessionStorage may be unavailable; the live event still works. */
  }
  window.dispatchEvent(new CustomEvent(MEMORY_FOCUS_EVENT, { detail: target }))
}

export function consumePendingMemoryFocus(): MemoryFocusTarget | null {
  if (typeof window === "undefined") return null
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    window.sessionStorage.removeItem(STORAGE_KEY)
    return normalizeTarget(JSON.parse(raw))
  } catch {
    return null
  }
}

export function subscribeMemoryFocus(handler: (target: MemoryFocusTarget) => void): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const target = normalizeTarget((event as CustomEvent<unknown>).detail)
    if (target) handler(target)
  }
  window.addEventListener(MEMORY_FOCUS_EVENT, listener)
  return () => window.removeEventListener(MEMORY_FOCUS_EVENT, listener)
}
