// Mirrors ha-core `ClaimRecord` (camelCase).
export interface ClaimRecord {
  id: string
  scopeType: string
  scopeId?: string | null
  claimType: string
  subject: string
  predicate: string
  object: string
  content: string
  tags: string[]
  confidence: number
  confidenceSource: string
  salience: number
  status: string
  validFrom?: string | null
  validUntil?: string | null
  createdAt: string
  updatedAt: string
}

export const PROFILE_CLAIM_TYPES = ["user_profile", "preference"] as const
export const PROJECT_CLAIM_TYPE = "project_fact"
export const PERSONAL_CLAIM_TYPES: ReadonlySet<string> = new Set(PROFILE_CLAIM_TYPES)

const DEFAULT_CLAIM_TYPES = [
  "user_profile",
  "preference",
  PROJECT_CLAIM_TYPE,
  "standing_rule",
  "reference",
  "task_pattern",
] as const

const DEFAULT_EVIDENCE_CLASSES = [
  "manual_correction",
  "user_confirmed",
  "explicit_user_statement",
  "project_artifact_fact",
  "assistant_inferred",
  "behavioral_pattern",
] as const

const DEFAULT_EVIDENCE_SOURCE_TYPES = [
  "session_message",
  "memory",
  "file",
  "tool_result",
  "url",
  "recap_facet",
  "manual",
] as const

const DEFAULT_CONFIDENCE_SOURCES = ["derived", "llm_adjusted", "user_confirmed"] as const

const DEFAULT_CLAIM_STATUSES = [
  "active",
  "superseded",
  "expired",
  "archived",
  "needs_review",
] as const

export interface ClaimSchemaMetadata {
  claimTypes: string[]
  profileClaimTypes: string[]
  projectClaimType: string
  evidenceClasses: string[]
  evidenceSourceTypes: string[]
  confidenceSources: string[]
  statuses: string[]
}

export const DEFAULT_CLAIM_SCHEMA: ClaimSchemaMetadata = {
  claimTypes: [...DEFAULT_CLAIM_TYPES],
  profileClaimTypes: [...PROFILE_CLAIM_TYPES],
  projectClaimType: PROJECT_CLAIM_TYPE,
  evidenceClasses: [...DEFAULT_EVIDENCE_CLASSES],
  evidenceSourceTypes: [...DEFAULT_EVIDENCE_SOURCE_TYPES],
  confidenceSources: [...DEFAULT_CONFIDENCE_SOURCES],
  statuses: [...DEFAULT_CLAIM_STATUSES],
}

export function normalizeClaimSchema(raw: Partial<ClaimSchemaMetadata> | null | undefined) {
  if (!raw) return DEFAULT_CLAIM_SCHEMA
  const claimTypes = nonEmptyStrings(raw.claimTypes, DEFAULT_CLAIM_SCHEMA.claimTypes)
  const profileClaimTypes = nonEmptyStrings(
    raw.profileClaimTypes,
    DEFAULT_CLAIM_SCHEMA.profileClaimTypes,
  ).filter((value) => claimTypes.includes(value))
  const projectClaimType =
    typeof raw.projectClaimType === "string" && raw.projectClaimType.trim()
      ? raw.projectClaimType.trim()
      : DEFAULT_CLAIM_SCHEMA.projectClaimType
  return {
    claimTypes,
    profileClaimTypes:
      profileClaimTypes.length > 0 ? profileClaimTypes : DEFAULT_CLAIM_SCHEMA.profileClaimTypes,
    projectClaimType,
    evidenceClasses: nonEmptyStrings(
      raw.evidenceClasses,
      DEFAULT_CLAIM_SCHEMA.evidenceClasses,
    ),
    evidenceSourceTypes: nonEmptyStrings(
      raw.evidenceSourceTypes,
      DEFAULT_CLAIM_SCHEMA.evidenceSourceTypes,
    ),
    confidenceSources: nonEmptyStrings(
      raw.confidenceSources,
      DEFAULT_CLAIM_SCHEMA.confidenceSources,
    ),
    statuses: nonEmptyStrings(raw.statuses, DEFAULT_CLAIM_SCHEMA.statuses),
  }
}

function nonEmptyStrings(values: unknown, fallback: string[]): string[] {
  if (!Array.isArray(values)) return [...fallback]
  const out: string[] = []
  for (const value of values) {
    if (typeof value !== "string") continue
    const trimmed = value.trim()
    if (trimmed && !out.includes(trimmed)) out.push(trimmed)
  }
  return out.length > 0 ? out : [...fallback]
}

function withAll(values: string[]): string[] {
  return ["all", ...values.filter((value) => value !== "all")]
}

export function claimTypeFilterValues(schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA) {
  const values = ["all", "profile", ...schema.claimTypes]
  return values.filter((value, index) => values.indexOf(value) === index)
}

export function evidenceClassFilterValues(schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA) {
  return withAll(schema.evidenceClasses)
}

export function evidenceSourceFilterValues(schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA) {
  return withAll(schema.evidenceSourceTypes)
}

export function confidenceSourceFilterValues(schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA) {
  return withAll(schema.confidenceSources)
}

export const CLAIM_TYPE_FILTER_VALUES = [
  "all",
  "profile",
  ...DEFAULT_CLAIM_TYPES,
] as const

export type ClaimTypeFilter = string

export function normalizeClaimTypeFilter(
  value: string | null | undefined,
  schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA,
): ClaimTypeFilter {
  const allowed = new Set(claimTypeFilterValues(schema))
  return allowed.has(value ?? "") ? (value as ClaimTypeFilter) : "all"
}
