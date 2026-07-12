export interface MemoryAuditActivityLike {
  createdAt: string
  id?: string
  key?: string
  kind?: string
}

export type MemoryAuditPageTaggedRecord<TMemory, TExperience, TDecision> =
  | { kind: "legacyMemory"; record: TMemory }
  | { kind: "experience"; record: TExperience }
  | { kind: "claimDecision"; record: TDecision }

export interface MemoryAuditPageItem<TMemory, TExperience, TDecision> {
  item: MemoryAuditPageTaggedRecord<TMemory, TExperience, TDecision>
}

export interface MemoryAuditActivityBuckets<TMemory, TExperience, TDecision> {
  memory: TMemory[]
  experience: TExperience[]
  decisions: TDecision[]
}

export function includeCrossSourceAudit(action: string | null | undefined): boolean {
  return !action || action === "all"
}

export function countMemoryAuditActivity(args: {
  memoryCount: number
  experienceCount: number
  decisionCount: number
  action: string | null | undefined
}): number {
  const includeCrossSource = includeCrossSourceAudit(args.action)
  return (
    args.memoryCount +
    (includeCrossSource ? args.experienceCount : 0) +
    (includeCrossSource ? args.decisionCount : 0)
  )
}

export function splitMemoryAuditPage<TMemory, TExperience, TDecision, TMappedDecision>(args: {
  items: MemoryAuditPageItem<TMemory, TExperience, TDecision>[]
  mapDecision: (decision: TDecision) => TMappedDecision
}): MemoryAuditActivityBuckets<TMemory, TExperience, TMappedDecision> {
  const buckets: MemoryAuditActivityBuckets<TMemory, TExperience, TMappedDecision> = {
    memory: [],
    experience: [],
    decisions: [],
  }
  for (const item of args.items) {
    switch (item.item.kind) {
      case "legacyMemory":
        buckets.memory.push(item.item.record)
        break
      case "experience":
        buckets.experience.push(item.item.record)
        break
      case "claimDecision":
        buckets.decisions.push(args.mapDecision(item.item.record))
        break
    }
  }
  return buckets
}

function auditTimeValue(value: string): number {
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

function auditSourceRank(entry: MemoryAuditActivityLike): number {
  switch (entry.kind) {
    case "decision":
    case "claim":
      return 0
    case "experience":
    case "experience_event":
      return 1
    case "memory":
    case "memory_event":
      return 2
    default:
      return 3
  }
}

function auditStableId(entry: MemoryAuditActivityLike): string {
  return entry.key || entry.id || ""
}

function compareAuditActivity(a: MemoryAuditActivityLike, b: MemoryAuditActivityLike): number {
  const timeDiff = auditTimeValue(b.createdAt) - auditTimeValue(a.createdAt)
  if (timeDiff !== 0) return timeDiff

  const sourceDiff = auditSourceRank(a) - auditSourceRank(b)
  if (sourceDiff !== 0) return sourceDiff

  const aId = auditStableId(a)
  const bId = auditStableId(b)
  if (aId < bId) return -1
  if (aId > bId) return 1
  return 0
}

export function mergeMemoryAuditActivity<T extends MemoryAuditActivityLike>(args: {
  memory: T[]
  experience: T[]
  decisions: T[]
  action: string | null | undefined
}): T[] {
  const includeCrossSource = includeCrossSourceAudit(args.action)
  return [
    ...args.memory,
    ...(includeCrossSource ? args.experience : []),
    ...(includeCrossSource ? args.decisions : []),
  ].sort(compareAuditActivity)
}
