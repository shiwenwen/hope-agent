import { fileKindOf, isPreviewableKind } from "@/lib/fileKind"
import type { FileRuntime, WorkspaceAccess } from "@/lib/transport"
import type {
  FileAction,
  FileCapability,
  FileCapabilityReason,
  FileCapabilitySet,
  FileTarget,
} from "./types"
import { DEFAULT_MAX_TEXT_EDIT_MB, MEBIBYTE_BYTES } from "@/lib/filesystemConfig"

const ENABLED: FileCapability = { state: "enabled" }
const disabled = (reason: FileCapabilityReason): FileCapability => ({ state: "disabled", reason })

const ALL_ACTIONS: FileAction[] = [
  "preview",
  "open",
  "download",
  "reveal",
  "edit",
  "remove",
  "rename",
  "delete",
  "createFile",
  "createFolder",
  "upload",
  "saveAs",
]

function emptySet(): FileCapabilitySet {
  return Object.fromEntries(
    ALL_ACTIONS.map((action) => [action, disabled("not_supported")]),
  ) as FileCapabilitySet
}

export function fileTargetName(target: FileTarget): string {
  switch (target.kind) {
    case "clientDraft":
      return target.draft.file.name
    case "workspace":
    case "sessionPath":
      return target.name
    case "media":
      return target.item.name
    case "knowledgeNote":
      return target.path.split("/").pop() || target.path
    case "artifact":
      return target.name
  }
}

export function fileTargetKind(target: FileTarget) {
  switch (target.kind) {
    case "clientDraft":
      return fileKindOf(target.draft.file.name, target.draft.file.type)
    case "workspace":
    case "sessionPath":
      return fileKindOf(target.name, target.mime, target.language)
    case "media":
      return fileKindOf(target.item.name, target.item.mimeType)
    case "knowledgeNote":
      return fileKindOf(target.path, "text/markdown")
    case "artifact":
      return fileKindOf(target.name, "text/html")
  }
}

function workspaceWriteCapability(access?: WorkspaceAccess): FileCapability {
  switch (access?.writeState) {
    case "enabled":
      return ENABLED
    case "remote_writes_disabled":
      return { state: "guided", reason: "remote_writes_disabled" }
    case "project_archived":
      return disabled("project_archived")
    case "scope_read_only":
    default:
      return disabled("scope_read_only")
  }
}

/** Pure capability matrix. Execution-time authorization remains backend-owned. */
export function resolveFileCapabilities(
  target: FileTarget,
  runtime: FileRuntime,
  workspaceAccess?: WorkspaceAccess,
  maxTextEditBytes: number = DEFAULT_MAX_TEXT_EDIT_MB * MEBIBYTE_BYTES,
): FileCapabilitySet {
  const result = emptySet()
  const kind = fileTargetKind(target)
  result.preview = isPreviewableKind(kind) ? ENABLED : disabled("not_previewable")
  result.open = ENABLED
  result.download = ENABLED
  result.reveal = runtime.canReveal ? ENABLED : disabled("reveal_unavailable")

  if (target.kind === "clientDraft") {
    result.preview = ENABLED
    result.reveal = disabled("reveal_unavailable")
    result.remove = ENABLED
    if (target.draft.file.size > maxTextEditBytes) {
      result.edit = disabled("too_large")
    } else if (["code", "text", "markdown"].includes(kind)) {
      result.edit = ENABLED
    }
    return result
  }

  if (target.kind === "workspace") {
    const write = workspaceWriteCapability(workspaceAccess)
    if (target.isDirectory) {
      result.preview = disabled("not_previewable")
      result.open = disabled("not_supported")
      result.download = disabled("not_supported")
      result.reveal = disabled("not_supported")
    }
    result.edit = target.isDirectory
      ? disabled("not_supported")
      : write.state === "disabled"
        ? write
        : target.sizeBytes != null && target.sizeBytes > maxTextEditBytes
          ? disabled("too_large")
          : ["code", "text", "markdown"].includes(kind)
            ? write
            : disabled("binary")
    result.rename = write
    result.delete = write
    result.createFile = write
    result.createFolder = write
    result.upload = write
    result.saveAs = target.isDirectory ? disabled("not_supported") : result.edit
  }

  if (target.kind === "knowledgeNote") {
    // Note mutations stay delegated to the knowledge service. The adapter only
    // exposes read-side actions until NoteEditor supplies its own live access.
    result.preview = ENABLED
    result.edit = ENABLED
    result.saveAs = ENABLED
    result.reveal = runtime.canReveal ? ENABLED : disabled("reveal_unavailable")
  }

  if (target.kind === "artifact") {
    result.preview = ENABLED
    result.edit = disabled("not_supported")
  }

  return result
}
