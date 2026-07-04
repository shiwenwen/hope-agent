/**
 * Type definitions for the Project feature.
 *
 * Mirrors `crates/ha-core/src/project/types.rs` with serde camelCase naming.
 */

export interface Project {
  id: string
  name: string
  description?: string | null
  /** Custom instructions appended to the system prompt of every session in the project. */
  instructions?: string | null
  /** Optional logo stored as a `data:image/...;base64,...` URL. */
  logo?: string | null
  /** Tailwind-ish color name (e.g. "amber", "violet"). */
  color?: string | null
  defaultAgentId?: string | null
  defaultModelId?: string | null
  /**
   * Default working directory for sessions in this project. Used as a
   * fallback when the session itself has no `workingDir` set.
   */
  workingDir?: string | null
  /** Unix milliseconds. */
  createdAt: number
  updatedAt: number
  /** Sidebar sort key. Lower values render earlier. */
  sortOrder: number
  archived: boolean
}

/** Project enriched with counts that drive the sidebar badges. */
export interface ProjectMeta extends Project {
  sessionCount: number
  unreadCount: number
  memoryCount: number
}

export interface CreateProjectInput {
  name: string
  description?: string | null
  instructions?: string | null
  /** Data URL (e.g. `data:image/webp;base64,...`). */
  logo?: string | null
  color?: string | null
  defaultAgentId?: string | null
  defaultModelId?: string | null
  /** Optional default working directory for sessions in this project. */
  workingDir?: string | null
}

/**
 * Patch DTO. `undefined` means "don't touch this field"; empty string is
 * treated by the backend as "clear this field".
 */
export interface UpdateProjectInput {
  name?: string
  description?: string
  instructions?: string
  /** Data URL, or empty string to clear the existing logo. */
  logo?: string
  color?: string
  defaultAgentId?: string
  defaultModelId?: string
  /** Empty string clears the project default working directory. */
  workingDir?: string
  archived?: boolean
}
