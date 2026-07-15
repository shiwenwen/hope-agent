// Mirrors the Rust `PlanIndexEntry` / `PlanIndexFilter` types from
// `crates/ha-core/src/plan/index.rs`. Single source of truth for the global
// Plans view and any other surface that consumes the cross-session plan
// index (e.g. dashboard stats list, @plan mention expansion).

import type { PlanModeState } from "@/components/chat/plan-mode/usePlanMode"

// Re-export under the alias used by this module's API for self-documentation.
export type PlanModeStateString = PlanModeState

export interface PlanIndexEntry {
  sessionId: string
  sessionShortId: string
  sessionTitle: string | null
  agentId: string
  projectId: string | null
  planFilePath: string
  state: PlanModeState
  title: string | null
  createdAt: string
  updatedAt: string
  sessionUpdatedAt: string | null
  versionCount: number
  executingStartedAt: string | null
  completedAt: string | null
  orphan: boolean
}

export interface PlanIndexFilter {
  agentId?: string | null
  sessionId?: string | null
  projectId?: string | null
  state?: PlanModeState | "" | null
  updatedAfter?: string | null
}

export interface PlanVersionInfoTs {
  version: number
  filePath: string
  modifiedAt: string
  isCurrent: boolean
}

export interface PlanMentionResolution {
  sessionId: string
  agentId: string
  filePath: string
  version: number
  title: string | null
}
