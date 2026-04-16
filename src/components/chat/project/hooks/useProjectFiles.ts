/**
 * Manages the shared file list for a specific project.
 *
 * Consumers pass `projectId` (nullable — hook stays idle when null) and get
 * back the file array plus upload / rename / delete helpers. Subscribes to
 * `project:file_*` events so uploads from other tabs show up immediately.
 */

import { useCallback, useEffect, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ProjectFile } from "@/types/project"
import { MAX_PROJECT_FILE_BYTES } from "@/types/project"

export interface UseProjectFilesReturn {
  files: ProjectFile[]
  loading: boolean
  error: string | null
  reloadFiles: () => Promise<void>
  uploadFile: (file: File) => Promise<ProjectFile | null>
  deleteFile: (fileId: string) => Promise<boolean>
  renameFile: (fileId: string, name: string) => Promise<boolean>
}

export function useProjectFiles(projectId: string | null): UseProjectFilesReturn {
  const [files, setFiles] = useState<ProjectFile[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const projectIdRef = useRef(projectId)
  projectIdRef.current = projectId

  const reloadFiles = useCallback(async () => {
    const pid = projectIdRef.current
    if (!pid) {
      setFiles([])
      return
    }
    setLoading(true)
    setError(null)
    try {
      const data = await getTransport().call<ProjectFile[]>("list_project_files_cmd", {
        projectId: pid,
      })
      setFiles(Array.isArray(data) ? data : [])
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.warn("useProjectFiles", "reloadFiles failed", msg)
      setError(msg)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void reloadFiles()
  }, [projectId, reloadFiles])

  // Refresh on upload/delete events for the active project.
  useEffect(() => {
    const transport = getTransport()
    const unsubs = ["project:file_uploaded", "project:file_deleted"].map((name) =>
      transport.listen(name, (payload: unknown) => {
        const pid = projectIdRef.current
        if (!pid) return
        const p = payload as { projectId?: string } | null
        if (!p || p.projectId !== pid) return
        void reloadFiles()
      }),
    )
    return () => {
      for (const u of unsubs) u()
    }
  }, [reloadFiles])

  const uploadFile = useCallback(
    async (file: File): Promise<ProjectFile | null> => {
      const pid = projectIdRef.current
      if (!pid) return null

      if (file.size > MAX_PROJECT_FILE_BYTES) {
        setError(
          `File too large: ${(file.size / 1024 / 1024).toFixed(1)} MB (max ${(MAX_PROJECT_FILE_BYTES / 1024 / 1024).toFixed(0)} MB)`,
        )
        return null
      }

      try {
        const buffer = await file.arrayBuffer()
        const data = getTransport().prepareFileData(buffer, file.type || "application/octet-stream")

        const result = await getTransport().call<ProjectFile>("upload_project_file_cmd", {
          projectId: pid,
          fileName: file.name,
          mimeType: file.type || undefined,
          data,
        })
        // Optimistic prepend; the EventBus listener will reconcile shortly.
        setFiles((prev) => [result, ...prev.filter((f) => f.id !== result.id)])
        return result
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e)
        logger.warn("useProjectFiles", "uploadFile failed", msg)
        setError(msg)
        return null
      }
    },
    [],
  )

  const deleteFile = useCallback(
    async (fileId: string): Promise<boolean> => {
      const pid = projectIdRef.current
      if (!pid) return false
      try {
        await getTransport().call("delete_project_file_cmd", {
          projectId: pid,
          fileId,
        })
        setFiles((prev) => prev.filter((f) => f.id !== fileId))
        return true
      } catch (e) {
        logger.warn("useProjectFiles", "deleteFile failed", e)
        return false
      }
    },
    [],
  )

  const renameFile = useCallback(
    async (fileId: string, name: string): Promise<boolean> => {
      const pid = projectIdRef.current
      if (!pid) return false
      try {
        await getTransport().call("rename_project_file_cmd", {
          projectId: pid,
          fileId,
          name,
        })
        setFiles((prev) =>
          prev.map((f) => (f.id === fileId ? { ...f, name } : f)),
        )
        return true
      } catch (e) {
        logger.warn("useProjectFiles", "renameFile failed", e)
        return false
      }
    },
    [],
  )

  return {
    files,
    loading,
    error,
    reloadFiles,
    uploadFile,
    deleteFile,
    renameFile,
  }
}
