import type { Transport, WorkspaceAccess } from "@/lib/transport"
import type { PreviewSource } from "./previewSource"
import {
  mediaPreviewSource,
  pathPreviewSource,
  stagedFilePreviewSource,
  workspacePreviewSource,
} from "./previewSource"
import { downloadStagedFile, openStagedFile } from "./stagedFileActions"
import { resolveFileCapabilities } from "./fileCapabilities"
import type { FileAction, FileActionInput, FileCapabilitySet, FileTarget } from "./types"
import type { FilesystemConfig } from "@/lib/filesystemConfig"
import { MEBIBYTE_BYTES } from "@/lib/filesystemConfig"

export interface FileActionContext {
  transport: Transport
  sessionId?: string | null
  workspaceAccess?: WorkspaceAccess
  objectUrl?: string
  onPreview?: (target: FileTarget) => void
  onEdit?: (target: FileTarget) => void
  onRemove?: (target: FileTarget) => void
  filesystemConfig?: FilesystemConfig
  workspaceOperations?: WorkspaceFileOperations
}

export interface WorkspaceFileOperations {
  createFile(dir: string, name: string): Promise<boolean>
  createFolder(dir: string, name: string): Promise<boolean>
  rename(path: string, toPath: string): Promise<boolean>
  remove(path: string, recursive: boolean): Promise<boolean>
  uploadInto(dir: string, files: File[]): Promise<boolean>
  saveAs(path: string, content: string): Promise<{ status: string }>
}

export interface FileResourceAdapter<T extends FileTarget> {
  capabilities(target: T, context: FileActionContext): Promise<FileCapabilitySet>
  previewSource(target: T, context: FileActionContext): PreviewSource
  run(
    target: T,
    action: FileAction,
    context: FileActionContext,
    input?: FileActionInput,
  ): Promise<boolean>
}

type TargetOf<K extends FileTarget["kind"]> = Extract<FileTarget, { kind: K }>

function commonCapabilities<T extends FileTarget>(
  target: T,
  context: FileActionContext,
): Promise<FileCapabilitySet> {
  return Promise.resolve(
    resolveFileCapabilities(
      target,
      context.transport.fileRuntime(),
      context.workspaceAccess,
      context.filesystemConfig?.maxTextEditMb != null
        ? context.filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES
        : undefined,
    ),
  )
}

const clientDraftAdapter: FileResourceAdapter<TargetOf<"clientDraft">> = {
  capabilities: commonCapabilities,
  previewSource: (target, context) => {
    if (!context.objectUrl) throw new Error("client draft preview requires an object URL lease")
    return stagedFilePreviewSource(
      target.draft.file,
      context.objectUrl,
      context.filesystemConfig?.maxTextPreviewMb != null
        ? context.filesystemConfig.maxTextPreviewMb * MEBIBYTE_BYTES
        : undefined,
      context.filesystemConfig?.maxDocumentPreviewMb != null
        ? context.filesystemConfig.maxDocumentPreviewMb * MEBIBYTE_BYTES
        : undefined,
    )
  },
  async run(target, action, context) {
    if (action === "preview") context.onPreview?.(target)
    else if (action === "open") openStagedFile(target.draft.file)
    else if (action === "download") downloadStagedFile(target.draft.file)
    else if (action === "edit") context.onEdit?.(target)
    else if (action === "remove") context.onRemove?.(target)
    else return false
    return true
  },
}

const workspaceAdapter: FileResourceAdapter<TargetOf<"workspace">> = {
  async capabilities(target, context) {
    const access =
      context.workspaceAccess ??
      (await context.transport.getWorkspaceAccess({ scope: target.scope, scopeId: target.scopeId }))
    return resolveFileCapabilities(
      target,
      context.transport.fileRuntime(),
      access,
      context.filesystemConfig?.maxTextEditMb != null
        ? context.filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES
        : undefined,
    )
  },
  previewSource: (target, context) => workspacePreviewSource(target, context.transport),
  async run(target, action, context, input) {
    const args = {
      scope: target.scope,
      scopeId: target.scopeId,
      path: target.relPath,
      name: target.name,
    }
    if (action === "preview") context.onPreview?.(target)
    else if (action === "open") await context.transport.openWorkspaceFile(args)
    else if (action === "download") await context.transport.downloadWorkspaceFile(args)
    else if (action === "reveal") await context.transport.revealWorkspaceFile(args)
    else if (action === "edit") context.onEdit?.(target)
    else if (action === "remove") context.onRemove?.(target)
    else {
      const operations = context.workspaceOperations
      if (!operations) return false
      if (action === "rename") {
        if (!input?.toPath) throw new Error("rename requires toPath")
        return operations.rename(input.path ?? target.relPath, input.toPath)
      }
      if (action === "delete") {
        return operations.remove(
          input?.path ?? target.relPath,
          input?.recursive ?? target.isDirectory === true,
        )
      }
      if (action === "createFile" || action === "createFolder") {
        if (!input?.name) throw new Error(`${action} requires name`)
        const dir = input.dirPath ?? (target.isDirectory ? target.relPath : "")
        return action === "createFile"
          ? operations.createFile(dir, input.name)
          : operations.createFolder(dir, input.name)
      }
      if (action === "upload") {
        if (!input?.files?.length) throw new Error("upload requires files")
        return operations.uploadInto(
          input.dirPath ?? (target.isDirectory ? target.relPath : ""),
          input.files,
        )
      }
      if (action === "saveAs") {
        if (!input?.path || input.content == null)
          throw new Error("saveAs requires path and content")
        const outcome = await operations.saveAs(input.path, input.content)
        return outcome.status === "saved"
      }
      return false
    }
    return true
  },
}

const sessionPathAdapter: FileResourceAdapter<TargetOf<"sessionPath">> = {
  capabilities: commonCapabilities,
  previewSource: (target, context) =>
    pathPreviewSource(
      target.path,
      target.name,
      target.sessionId ?? context.sessionId,
      target.mime,
      target.language,
      context.transport,
    ),
  async run(target, action, context) {
    const sessionId = target.sessionId ?? context.sessionId
    if (action === "preview") context.onPreview?.(target)
    else if (action === "open") await context.transport.openFilePath(target.path, { sessionId })
    else if (action === "download") {
      await context.transport.downloadFilePath(target.path, { sessionId, filename: target.name })
    } else if (action === "reveal") {
      await context.transport.call("reveal_in_folder", { path: target.path })
    } else return false
    return true
  },
}

const mediaAdapter: FileResourceAdapter<TargetOf<"media">> = {
  capabilities: commonCapabilities,
  previewSource: (target, context) =>
    mediaPreviewSource(
      target.item,
      context.sessionId,
      context.transport,
      context.filesystemConfig?.maxTextPreviewMb != null
        ? context.filesystemConfig.maxTextPreviewMb * MEBIBYTE_BYTES
        : undefined,
    ),
  async run(target, action, context) {
    if (action === "preview") context.onPreview?.(target)
    else if (action === "open") await context.transport.openMedia(target.item)
    else if (action === "download") await context.transport.downloadMedia(target.item)
    else if (action === "reveal") await context.transport.revealMedia(target.item)
    else return false
    return true
  },
}

const knowledgeNoteAdapter: FileResourceAdapter<TargetOf<"knowledgeNote">> = {
  capabilities: commonCapabilities,
  previewSource: (target, context) => ({
    name: target.path.split("/").pop() || target.path,
    mime: "text/markdown",
    displayPath: target.path,
    async readText() {
      const note = await context.transport.call<{ content: string; contentHash?: string }>(
        "kb_note_read_cmd",
        { kbId: target.kbId, path: target.path },
      )
      return {
        relPath: target.path,
        content: note.content,
        isBinary: false,
        mime: "text/markdown",
        totalLines: note.content.split("\n").length,
        sizeBytes: new TextEncoder().encode(note.content).byteLength,
        truncated: false,
        contentHash: note.contentHash ?? null,
        isUtf8: true,
        lineEnding: "lf",
        hasUtf8Bom: false,
      }
    },
    async extractDoc() {
      throw new Error("knowledge notes are text documents")
    },
    async rawUrl() {
      return null
    },
  }),
  async run(target, action, context) {
    if (action === "preview" || action === "open") context.onPreview?.(target)
    else if (action === "edit") context.onEdit?.(target)
    else if (action === "reveal") {
      const path = await context.transport.call<string>("kb_file_resolve_cmd", {
        kbId: target.kbId,
        path: target.path,
      })
      await context.transport.call("reveal_in_folder", { path })
    } else if (action === "download") {
      const note = await context.transport.call<{ content: string }>("kb_note_read_cmd", {
        kbId: target.kbId,
        path: target.path,
      })
      downloadStagedFile(
        new File([note.content], target.path.split("/").pop() || "note.md", {
          type: "text/markdown",
        }),
      )
    } else return false
    return true
  },
}

export const fileResourceAdapters = {
  clientDraft: clientDraftAdapter,
  workspace: workspaceAdapter,
  sessionPath: sessionPathAdapter,
  media: mediaAdapter,
  knowledgeNote: knowledgeNoteAdapter,
}

export function fileResourceAdapterFor<T extends FileTarget>(target: T): FileResourceAdapter<T> {
  return fileResourceAdapters[target.kind] as FileResourceAdapter<T>
}
