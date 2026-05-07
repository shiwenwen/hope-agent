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
  emoji?: string | null
  /** Optional logo stored as a `data:image/...;base64,...` URL. Takes precedence over `emoji` in the UI. */
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
  archived: boolean
}

/** Project enriched with counts that drive the sidebar badges. */
export interface ProjectMeta extends Project {
  sessionCount: number
  unreadCount: number
  fileCount: number
  memoryCount: number
}

export interface ProjectFile {
  id: string
  projectId: string
  /** User-editable display name; defaults to `originalFilename`. */
  name: string
  originalFilename: string
  mimeType?: string | null
  sizeBytes: number
  /** Storage path relative to `~/.hope-agent/projects/`. */
  filePath: string
  extractedPath?: string | null
  extractedChars?: number | null
  summary?: string | null
  createdAt: number
  updatedAt: number
}

export interface CreateProjectInput {
  name: string
  description?: string | null
  instructions?: string | null
  emoji?: string | null
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
  emoji?: string
  /** Data URL, or empty string to clear the existing logo. */
  logo?: string
  color?: string
  defaultAgentId?: string
  defaultModelId?: string
  /** Empty string clears the project default working directory. */
  workingDir?: string
  archived?: boolean
}

/** Maximum size of a single project file upload (matches `MAX_PROJECT_FILE_BYTES`). */
export const MAX_PROJECT_FILE_BYTES = 20 * 1024 * 1024
