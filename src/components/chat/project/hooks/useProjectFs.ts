/**
 * Workspace-scoped filesystem data layer for the project file browser.
 *
 * Lazily loads one directory level at a time (keyed by `/`-relative path) and
 * exposes CRUD that refresh the affected directories. Subscribes to
 * `project:fs_changed` so the two mount points (Files tab + right panel) and
 * agent-produced files stay in sync.
 */

import { useCallback, useEffect, useMemo, useState } from "react"

import { useTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  ExtractedContent,
  FileSearchResponse,
  FileTextContent,
  FileWriteOutcome,
  ProjectFsScope,
  WorkspaceAccess,
  WorkspaceEntry,
  WorkspaceListing,
} from "@/lib/transport"

export interface DirState {
  entries: WorkspaceEntry[]
  loading: boolean
  error: string | null
}

export interface ProjectFsApi {
  scope: ProjectFsScope
  available: boolean
  access: WorkspaceAccess | null
  accessLoading: boolean
  refreshAccess: () => Promise<void>
  getDir: (dir: string) => DirState | undefined
  loadDir: (dir: string) => Promise<void>
  refreshDir: (dir: string) => Promise<void>
  readFile: (path: string) => Promise<FileTextContent>
  extractDoc: (path: string) => Promise<ExtractedContent>
  searchFiles: (q: string, limit?: number) => Promise<FileSearchResponse>
  rawUrl: (path: string, download?: boolean) => Promise<string | null>
  createFile: (dir: string, name: string) => Promise<boolean>
  createFolder: (dir: string, name: string) => Promise<boolean>
  rename: (path: string, toPath: string) => Promise<boolean>
  remove: (path: string, recursive: boolean) => Promise<boolean>
  uploadInto: (dir: string, files: File[]) => Promise<boolean>
  writeText: (path: string, content: string, expectedFileHash: string) => Promise<FileWriteOutcome>
  saveAs: (path: string, content: string) => Promise<FileWriteOutcome>
}

function parentOf(rel: string): string {
  const trimmed = rel.replace(/\/+$/, "")
  const i = trimmed.lastIndexOf("/")
  return i >= 0 ? trimmed.slice(0, i) : ""
}

function joinRel(dir: string, name: string): string {
  const d = dir.replace(/\/+$/, "")
  return d ? `${d}/${name}` : name
}

export function useProjectFs(
  scope: "session" | "project" | "path",
  scopeId: string | null,
): ProjectFsApi {
  const transport = useTransport()
  const [dirs, setDirs] = useState<Record<string, DirState>>({})
  const [access, setAccess] = useState<WorkspaceAccess | null>(null)
  const [accessLoading, setAccessLoading] = useState(false)

  // Reset the cached directories when the scope target changes, using the
  // setState-during-render pattern (React-recommended over an effect).
  const scopeKey = `${scope}:${scopeId ?? ""}`
  const [trackedKey, setTrackedKey] = useState(scopeKey)
  if (scopeKey !== trackedKey) {
    setTrackedKey(scopeKey)
    setDirs({})
    setAccess(null)
  }

  const scopeArg = useMemo<ProjectFsScope>(
    () => ({ scope, scopeId: scopeId ?? "" }),
    [scope, scopeId],
  )

  const refreshAccess = useCallback(async () => {
    if (!scopeId) {
      setAccess(null)
      return
    }
    setAccessLoading(true)
    try {
      setAccess(await transport.getWorkspaceAccess(scopeArg))
    } catch (e) {
      logger.warn("chat", "useProjectFs", "capabilities failed", e)
      setAccess(null)
    } finally {
      setAccessLoading(false)
    }
  }, [scopeArg, scopeId, transport])

  useEffect(() => {
    void refreshAccess()
    const offConfig = transport.listen("config:changed", () => void refreshAccess())
    const offResync = transport.listen("transport:event-stream-resync-required", () => {
      void refreshAccess()
    })
    return () => {
      offConfig()
      offResync()
    }
  }, [refreshAccess, transport])

  const loadDir = useCallback(
    async (dir: string) => {
      if (!scopeId) return
      setDirs((prev) => ({
        ...prev,
        [dir]: { entries: prev[dir]?.entries ?? [], loading: true, error: null },
      }))
      try {
        const res = await transport.call<WorkspaceListing>("project_fs_list", {
          scope,
          scopeId,
          path: dir,
        })
        setDirs((prev) => ({
          ...prev,
          [dir]: { entries: res.entries, loading: false, error: null },
        }))
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e)
        logger.warn("chat", "useProjectFs", "loadDir failed", msg)
        setDirs((prev) => ({
          ...prev,
          [dir]: { entries: prev[dir]?.entries ?? [], loading: false, error: msg },
        }))
      }
    },
    [scope, scopeId, transport],
  )

  const refreshDir = useCallback(
    async (dir: string) => {
      await loadDir(dir)
    },
    [loadDir],
  )

  // Cross-view / agent-write sync: re-fetch a directory we've already loaded
  // when something changes it elsewhere.
  useEffect(() => {
    if (!scopeId) return
    return transport.listen("project:fs_changed", (payload: unknown) => {
      const p = payload as { scope?: string; scopeId?: string; dir?: string } | null
      if (!p || p.scope !== scope || p.scopeId !== scopeId) return
      void loadDir(p.dir ?? "")
    })
  }, [scope, scopeId, loadDir, transport])

  const mutate = useCallback(
    async (command: string, extra: Record<string, unknown>): Promise<boolean> => {
      if (!scopeId) return false
      await transport.call(command, { scope, scopeId, ...extra })
      return true
    },
    [scope, scopeId, transport],
  )

  const readFile = useCallback(
    async (path: string): Promise<FileTextContent> => {
      if (!scopeId) throw new Error("no workspace")
      return transport.call<FileTextContent>("project_fs_read_text", { scope, scopeId, path })
    },
    [scope, scopeId, transport],
  )

  const extractDoc = useCallback(
    async (path: string): Promise<ExtractedContent> => {
      if (!scopeId) throw new Error("no workspace")
      return transport.call<ExtractedContent>("project_fs_extract", { scope, scopeId, path })
    },
    [scope, scopeId, transport],
  )

  const searchFiles = useCallback(
    async (q: string, limit?: number): Promise<FileSearchResponse> => {
      if (!scopeId) throw new Error("no workspace")
      return transport.call<FileSearchResponse>("project_fs_search", {
        scope,
        scopeId,
        q,
        limit,
      })
    },
    [scope, scopeId, transport],
  )

  const rawUrl = useCallback(
    async (path: string, download?: boolean): Promise<string | null> => {
      if (!scopeId) return null
      return transport.projectFsRawUrl({ scope, scopeId, path, download })
    },
    [scope, scopeId, transport],
  )

  const createFile = useCallback(
    async (dir: string, name: string): Promise<boolean> => {
      try {
        const ok = await mutate("project_fs_write_text", {
          path: joinRel(dir, name),
          content: "",
          createOnly: true,
        })
        if (ok) await loadDir(dir)
        return ok
      } catch (e) {
        logger.warn("chat", "useProjectFs", "createFile failed", e)
        return false
      }
    },
    [mutate, loadDir],
  )

  const createFolder = useCallback(
    async (dir: string, name: string): Promise<boolean> => {
      try {
        const ok = await mutate("project_fs_mkdir", { path: joinRel(dir, name) })
        if (ok) await loadDir(dir)
        return ok
      } catch (e) {
        logger.warn("chat", "useProjectFs", "createFolder failed", e)
        return false
      }
    },
    [mutate, loadDir],
  )

  const rename = useCallback(
    async (path: string, toPath: string): Promise<boolean> => {
      try {
        const ok = await mutate("project_fs_rename", { fromPath: path, toPath })
        if (ok) {
          await loadDir(parentOf(path))
          if (parentOf(toPath) !== parentOf(path)) await loadDir(parentOf(toPath))
        }
        return ok
      } catch (e) {
        logger.warn("chat", "useProjectFs", "rename failed", e)
        return false
      }
    },
    [mutate, loadDir],
  )

  const remove = useCallback(
    async (path: string, recursive: boolean): Promise<boolean> => {
      try {
        const ok = await mutate("project_fs_delete", { path, recursive })
        if (ok) await loadDir(parentOf(path))
        return ok
      } catch (e) {
        logger.warn("chat", "useProjectFs", "remove failed", e)
        return false
      }
    },
    [mutate, loadDir],
  )

  const writeText = useCallback(
    async (path: string, content: string, expectedFileHash: string): Promise<FileWriteOutcome> => {
      if (!scopeId) throw new Error("no workspace")
      return transport.call<FileWriteOutcome>("project_fs_write_text", {
        scope,
        scopeId,
        path,
        content,
        expectedFileHash,
      })
    },
    [scope, scopeId, transport],
  )

  const saveAs = useCallback(
    async (path: string, content: string): Promise<FileWriteOutcome> => {
      if (!scopeId) throw new Error("no workspace")
      return transport.call<FileWriteOutcome>("project_fs_write_text", {
        scope,
        scopeId,
        path,
        content,
        createOnly: true,
      })
    },
    [scope, scopeId, transport],
  )

  const uploadInto = useCallback(
    async (dir: string, files: File[]): Promise<boolean> => {
      if (!scopeId) return false
      try {
        let next = 0
        const workers = Array.from({ length: Math.min(3, files.length) }, async () => {
          while (next < files.length) {
            const file = files[next]
            next += 1
            await transport.projectFsUpload({
              scope,
              scopeId,
              dirPath: dir,
              data: file,
              fileName: file.name,
              mimeType: file.type || undefined,
            })
          }
        })
        await Promise.all(workers)
        await loadDir(dir)
        return true
      } catch (e) {
        logger.warn("chat", "useProjectFs", "uploadInto failed", e)
        return false
      }
    },
    [scope, scopeId, loadDir, transport],
  )

  const getDir = useCallback((dir: string) => dirs[dir], [dirs])

  // Memoize so the returned API keeps a stable identity across renders;
  // consumers depend on `fs` in effects (FilePreviewPane re-fetches + clears the
  // selection whenever it changes), so a fresh object each render would re-run
  // them on every unrelated directory load.
  return useMemo<ProjectFsApi>(
    () => ({
      scope: scopeArg,
      available: !!scopeId,
      access,
      accessLoading,
      refreshAccess,
      getDir,
      loadDir,
      refreshDir,
      readFile,
      extractDoc,
      searchFiles,
      rawUrl,
      createFile,
      createFolder,
      rename,
      remove,
      uploadInto,
      writeText,
      saveAs,
    }),
    [
      scopeArg,
      scopeId,
      access,
      accessLoading,
      refreshAccess,
      getDir,
      loadDir,
      refreshDir,
      readFile,
      extractDoc,
      searchFiles,
      rawUrl,
      createFile,
      createFolder,
      rename,
      remove,
      uploadInto,
      writeText,
      saveAs,
    ],
  )
}
