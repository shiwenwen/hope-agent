import type { MediaItem } from "@/types/chat"
import type { ArtifactExportFormat } from "@/lib/transport"

export interface DraftAttachment {
  id: string
  file: File
  acquisition: "paste" | "drop" | "picker"
  semanticSource: "upload" | "pasted_text"
  status: "ready" | "uploading" | "error"
  error?: string
}

let draftIdFallback = 0

export function createDraftAttachment(
  file: File,
  acquisition: DraftAttachment["acquisition"],
  semanticSource: DraftAttachment["semanticSource"] = "upload",
): DraftAttachment {
  draftIdFallback += 1
  return {
    id:
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `draft-${Date.now()}-${draftIdFallback}`,
    file,
    acquisition,
    semanticSource,
    status: "ready",
  }
}

/** A file identity independent of the current transport/runtime location. */
export type FileTarget =
  | {
      kind: "clientDraft"
      draft: DraftAttachment
      /** Forces a fresh object-URL lease when the same draft is reopened. */
      previewId: string
    }
  | {
      kind: "workspace"
      scope: "session" | "project" | "path"
      scopeId: string
      relPath: string
      name: string
      mime?: string | null
      language?: string | null
      sizeBytes?: number | null
      isDirectory?: boolean
      revealLines?: { start: number; end: number; nonce: number }
    }
  | {
      kind: "sessionPath"
      /** May be supplied by the ambient chat file-action context. */
      sessionId?: string | null
      path: string
      name: string
      mime?: string | null
      language?: string | null
      revealLines?: { start: number; end: number; nonce: number }
    }
  | { kind: "media"; item: MediaItem }
  | { kind: "knowledgeNote"; kbId: string; path: string; contentHash?: string }
  | {
      kind: "artifact"
      artifactId: string
      name: string
      projectPath?: string | null
    }

export type FileAction =
  | "preview"
  | "open"
  | "download"
  | "reveal"
  | "edit"
  | "remove"
  | "rename"
  | "delete"
  | "createFile"
  | "createFolder"
  | "upload"
  | "saveAs"

export type CapabilityState = "enabled" | "guided" | "disabled"

export type FileCapabilityReason =
  | "not_supported"
  | "not_previewable"
  | "not_utf8"
  | "binary"
  | "too_large"
  | "remote_writes_disabled"
  | "scope_read_only"
  | "project_archived"
  | "reveal_unavailable"

export interface FileCapability {
  state: CapabilityState
  reason?: FileCapabilityReason
}

export type FileCapabilitySet = Record<FileAction, FileCapability>

export interface FileActionInput {
  /** Validate capability and run guided handling without performing a mutation. */
  prepareOnly?: boolean
  toPath?: string
  dirPath?: string
  name?: string
  files?: File[]
  recursive?: boolean
  path?: string
  content?: string
  artifactFormat?: ArtifactExportFormat
}

export type FileActionRunResult = "executed" | "guided" | "disabled" | "failed"
