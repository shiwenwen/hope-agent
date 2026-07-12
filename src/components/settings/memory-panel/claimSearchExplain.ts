import type { ClaimRecord } from "./claimTypes"

export type ClaimListBackendSort =
  | "updated_desc"
  | "created_desc"
  | "created_asc"
  | "confidence_desc"
  | "confidence_asc"
  | "salience_desc"
  | "salience_asc"

export type ClaimListSortRuntimeMode = "best_match" | "recent_fallback" | "explicit_sort"

export type ClaimSearchRankSignalKind = "salience" | "confidence" | "updated" | "created"

export interface ClaimSearchRankSignal {
  kind: ClaimSearchRankSignalKind
  direction: "asc" | "desc"
  value: string | number
}

export type ClaimSearchMatchKind =
  | "content"
  | "triple"
  | "type"
  | "status"
  | "scope"
  | "tag"
  | "confidence"
  | "evidence"

export interface ClaimSearchMatch {
  kind: ClaimSearchMatchKind
}

export interface ClaimSearchDiagnostics {
  query: string
  runtimeMode: ClaimListSortRuntimeMode
  matches: ClaimSearchMatch[]
  rankSignals: ClaimSearchRankSignal[]
}

function normalizeSearchText(value: string | null | undefined): string {
  return (value ?? "").trim().toLocaleLowerCase()
}

function includesNeedle(value: string | null | undefined, needle: string): boolean {
  if (!needle) return false
  return normalizeSearchText(value).includes(needle)
}

function pushOnce(matches: ClaimSearchMatch[], kind: ClaimSearchMatchKind): void {
  if (!matches.some((match) => match.kind === kind)) matches.push({ kind })
}

export function claimListBackendSortArg(
  sort: string | null | undefined,
  query: string | null | undefined,
): ClaimListBackendSort | undefined {
  const normalized = (sort ?? "").trim()
  const hasQuery = (query ?? "").trim().length > 0
  if (normalized === "relevance") {
    return hasQuery ? undefined : "updated_desc"
  }
  if (
    normalized === "updated_desc" ||
    normalized === "created_desc" ||
    normalized === "created_asc" ||
    normalized === "confidence_desc" ||
    normalized === "confidence_asc" ||
    normalized === "salience_desc" ||
    normalized === "salience_asc"
  ) {
    return normalized
  }
  return hasQuery ? undefined : "updated_desc"
}

export function claimListSortRuntimeMode(
  sort: string | null | undefined,
  query: string | null | undefined,
): ClaimListSortRuntimeMode {
  const normalized = (sort ?? "").trim()
  const hasQuery = (query ?? "").trim().length > 0
  if (normalized === "relevance" || normalized === "") {
    return hasQuery ? "best_match" : "recent_fallback"
  }
  if (
    normalized === "updated_desc" ||
    normalized === "created_desc" ||
    normalized === "created_asc" ||
    normalized === "confidence_desc" ||
    normalized === "confidence_asc" ||
    normalized === "salience_desc" ||
    normalized === "salience_asc"
  ) {
    return "explicit_sort"
  }
  return hasQuery ? "best_match" : "recent_fallback"
}

export function explainClaimSearchMatch(
  claim: Pick<
    ClaimRecord,
    | "content"
    | "claimType"
    | "status"
    | "scopeType"
    | "scopeId"
    | "subject"
    | "predicate"
    | "object"
    | "confidenceSource"
    | "tags"
  >,
  query: string,
  scopeLabel: string,
): ClaimSearchMatch[] {
  const terms = normalizeSearchText(query).split(/\s+/).filter(Boolean)
  if (terms.length === 0) return []

  const matches: ClaimSearchMatch[] = []
  for (const term of terms) {
    if (includesNeedle(claim.content, term)) pushOnce(matches, "content")
    if (
      includesNeedle(claim.subject, term) ||
      includesNeedle(claim.predicate, term) ||
      includesNeedle(claim.object, term)
    ) {
      pushOnce(matches, "triple")
    }
    if (includesNeedle(claim.claimType, term)) pushOnce(matches, "type")
    if (includesNeedle(claim.status, term)) pushOnce(matches, "status")
    if (
      includesNeedle(claim.scopeType, term) ||
      includesNeedle(claim.scopeId, term) ||
      includesNeedle(scopeLabel, term)
    ) {
      pushOnce(matches, "scope")
    }
    if ((claim.tags ?? []).some((tag) => includesNeedle(tag, term))) pushOnce(matches, "tag")
    if (includesNeedle(claim.confidenceSource, term)) pushOnce(matches, "confidence")
  }

  if (matches.length === 0) matches.push({ kind: "evidence" })
  return matches
}

export function explainClaimSearchRankSignals(
  sort: string | null | undefined,
  query: string | null | undefined,
  claim: Pick<ClaimRecord, "salience" | "confidence" | "updatedAt" | "createdAt">,
): ClaimSearchRankSignal[] {
  const runtimeMode = claimListSortRuntimeMode(sort, query)
  const normalized = (sort ?? "").trim()

  if (runtimeMode === "best_match") {
    return [
      { kind: "salience", direction: "desc", value: claim.salience },
      { kind: "confidence", direction: "desc", value: claim.confidence },
      { kind: "updated", direction: "desc", value: claim.updatedAt },
    ]
  }

  if (runtimeMode === "recent_fallback") {
    return [{ kind: "updated", direction: "desc", value: claim.updatedAt }]
  }

  switch (normalized) {
    case "created_desc":
      return [
        { kind: "created", direction: "desc", value: claim.createdAt },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    case "created_asc":
      return [
        { kind: "created", direction: "asc", value: claim.createdAt },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    case "confidence_desc":
      return [
        { kind: "confidence", direction: "desc", value: claim.confidence },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    case "confidence_asc":
      return [
        { kind: "confidence", direction: "asc", value: claim.confidence },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    case "salience_desc":
      return [
        { kind: "salience", direction: "desc", value: claim.salience },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    case "salience_asc":
      return [
        { kind: "salience", direction: "asc", value: claim.salience },
        { kind: "updated", direction: "desc", value: claim.updatedAt },
      ]
    default:
      return [{ kind: "updated", direction: "desc", value: claim.updatedAt }]
  }
}

export function claimSearchDiagnostics(
  claim: Pick<
    ClaimRecord,
    | "content"
    | "claimType"
    | "status"
    | "scopeType"
    | "scopeId"
    | "subject"
    | "predicate"
    | "object"
    | "confidenceSource"
    | "tags"
    | "salience"
    | "confidence"
    | "updatedAt"
    | "createdAt"
  >,
  query: string | null | undefined,
  scopeLabel: string,
  sort: string | null | undefined,
): ClaimSearchDiagnostics | null {
  const normalizedQuery = normalizeSearchText(query)
  if (!normalizedQuery) return null
  return {
    query: normalizedQuery,
    runtimeMode: claimListSortRuntimeMode(sort, normalizedQuery),
    matches: explainClaimSearchMatch(claim, normalizedQuery, scopeLabel),
    rankSignals: explainClaimSearchRankSignals(sort, normalizedQuery, claim),
  }
}
