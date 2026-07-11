import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeViewOperation =
  | "loadSpaces"
  | "loadNotes"
  | "loadFolders"
  | "loadTags"
  | "searchNotes"
  | "saveNote"
  | "createSpace"
  | "createNote"
  | "createLinkedNote"
  | "reindexSpace"
  | "reindexNote"
  | "reindexFolder"
  | "renameMove"
  | "deleteNote"
  | "deleteFolder"
  | "revealNote"
  | "createFolder"
  | "updateSpace"
  | "syncExternalRaw"
  | "archiveSpace"
  | "deleteSpace"

export type KnowledgeViewFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeViewOperationErrorToast {
  title: string
  description?: string
}

export function knowledgeViewErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function isKnowledgeRemoteWriteBlocked(error: unknown): boolean {
  const msg = error instanceof Error ? error.message : String(error)
  return /allowremotewrites|remote file writes are disabled/i.test(msg)
}

export function knowledgeViewOperationErrorToast(
  operation: KnowledgeViewOperation,
  t: KnowledgeViewFeedbackTranslateFn,
  error: unknown,
  options: Record<string, unknown> = {},
): KnowledgeViewOperationErrorToast {
  const titleKey =
    isWriteOperation(operation) && isKnowledgeRemoteWriteBlocked(error)
      ? "knowledge.remoteWritesDisabled"
      : knowledgeViewOperationKey(operation)
  const title = t(titleKey, {
    ...options,
    defaultValue:
      titleKey === "knowledge.remoteWritesDisabled"
        ? "Remote file writing is off. Turn it on in Settings → Server → Allow remote file writes (on the server host)."
        : knowledgeViewOperationFallback(operation),
  })
  const detail = knowledgeViewErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.operationErrorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function isWriteOperation(operation: KnowledgeViewOperation): boolean {
  switch (operation) {
    case "saveNote":
    case "createSpace":
    case "createNote":
    case "createLinkedNote":
    case "renameMove":
    case "deleteNote":
    case "deleteFolder":
    case "createFolder":
    case "updateSpace":
    case "syncExternalRaw":
    case "archiveSpace":
    case "deleteSpace":
      return true
    default:
      return false
  }
}

function knowledgeViewOperationKey(operation: KnowledgeViewOperation): string {
  switch (operation) {
    case "loadSpaces":
      return "knowledge.errors.loadSpaces"
    case "loadNotes":
      return "knowledge.errors.loadNotes"
    case "loadFolders":
      return "knowledge.errors.loadFolders"
    case "loadTags":
      return "knowledge.errors.loadTags"
    case "searchNotes":
      return "knowledge.errors.searchNotes"
    case "saveNote":
      return "knowledge.errors.saveNote"
    case "createSpace":
      return "knowledge.errors.createSpace"
    case "createNote":
      return "knowledge.errors.createNote"
    case "createLinkedNote":
      return "knowledge.errors.createLinkedNote"
    case "reindexSpace":
      return "knowledge.errors.reindexSpace"
    case "reindexNote":
      return "knowledge.errors.reindexNote"
    case "reindexFolder":
      return "knowledge.errors.reindexFolder"
    case "renameMove":
      return "knowledge.renameMoveFailed"
    case "deleteNote":
      return "knowledge.errors.deleteNote"
    case "deleteFolder":
      return "knowledge.errors.deleteFolder"
    case "revealNote":
      return "knowledge.errors.revealNote"
    case "createFolder":
      return "knowledge.errors.createFolder"
    case "updateSpace":
      return "knowledge.errors.updateSpace"
    case "syncExternalRaw":
      return "knowledge.externalRawSyncFailed"
    case "archiveSpace":
      return "knowledge.errors.archiveSpace"
    case "deleteSpace":
      return "knowledge.errors.deleteSpace"
  }
}

function knowledgeViewOperationFallback(operation: KnowledgeViewOperation): string {
  switch (operation) {
    case "loadSpaces":
      return "Couldn't load knowledge spaces"
    case "loadNotes":
      return "Couldn't load notes"
    case "loadFolders":
      return "Couldn't load folders"
    case "loadTags":
      return "Couldn't load tags"
    case "searchNotes":
      return "Couldn't search notes"
    case "saveNote":
      return "Couldn't save note"
    case "createSpace":
      return "Couldn't create knowledge space"
    case "createNote":
      return "Couldn't create note"
    case "createLinkedNote":
      return "Couldn't create linked note"
    case "reindexSpace":
      return "Couldn't rebuild space index"
    case "reindexNote":
      return "Couldn't rebuild note index"
    case "reindexFolder":
      return "Couldn't rebuild folder index"
    case "renameMove":
      return "Couldn't rename or move item"
    case "deleteNote":
      return "Couldn't delete note"
    case "deleteFolder":
      return "Couldn't delete folder"
    case "revealNote":
      return "Couldn't reveal note"
    case "createFolder":
      return "Couldn't create folder"
    case "updateSpace":
      return "Couldn't update knowledge space"
    case "syncExternalRaw":
      return "Couldn't sync source snapshots"
    case "archiveSpace":
      return "Couldn't update archive status"
    case "deleteSpace":
      return "Couldn't delete knowledge space"
  }
}
