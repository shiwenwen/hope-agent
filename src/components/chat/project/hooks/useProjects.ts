/**
 * Loads and manages the project list.
 *
 * Wraps the `list_projects_cmd` / `create_project_cmd` / ... command surface
 * and transparently refreshes on EventBus `project:*` events so that any
 * mutation (from the current tab or another tab) reflows the UI within one
 * render.
 */

import { useCallback, useEffect, useRef, useState } from "react"
import type { MutableRefObject } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  CreateProjectInput,
  Project,
  ProjectMeta,
  UpdateProjectInput,
} from "@/types/project"

export interface UseProjectsReturn {
  projects: ProjectMeta[]
  loading: boolean
  initialLoading: boolean
  error: string | null
  reloadProjects: () => Promise<void>
  createProject: (input: CreateProjectInput) => Promise<Project | null>
  updateProject: (id: string, patch: UpdateProjectInput) => Promise<Project | null>
  deleteProject: (id: string) => Promise<boolean>
  archiveProject: (id: string, archived: boolean) => Promise<Project | null>
  reorderProjects: (projectIds: string[]) => Promise<void>
  moveSessionToProject: (sessionId: string, projectId: string | null) => Promise<void>
}

export function useProjects(
  options: {
    includeArchived?: boolean
    /** Currently-open session id, read at fetch time and forwarded to the
     *  backend so its unread is excluded from the owning project's badge. The
     *  caller is responsible for calling `reloadProjects` when it changes (the
     *  ref avoids re-creating the hook's callbacks on every session switch). */
    activeSessionIdRef?: MutableRefObject<string | null>
  } = {},
): UseProjectsReturn {
  const { includeArchived = false, activeSessionIdRef } = options

  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [initialLoading, setInitialLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Keep the latest args in a ref so the EventBus handler always reloads
  // with the current filter without triggering reload chains.
  const includeArchivedRef = useRef(includeArchived)
  includeArchivedRef.current = includeArchived

  // Monotonic request token: rapid reloads (e.g. a session switch fires one
  // reload from `handleSwitchSession` and another from the active-session
  // effect) can resolve out of order. Only the latest request commits, so a
  // slow earlier response can't overwrite the badge with a stale active-session
  // exclusion.
  const reloadSeqRef = useRef(0)

  const reloadProjects = useCallback(async () => {
    const seq = ++reloadSeqRef.current
    setLoading(true)
    setError(null)
    try {
      const data = await getTransport().call<ProjectMeta[]>("list_projects_cmd", {
        includeArchived: includeArchivedRef.current,
        activeSessionId: activeSessionIdRef?.current ?? undefined,
      })
      if (seq !== reloadSeqRef.current) return
      setProjects(Array.isArray(data) ? data : [])
    } catch (e) {
      if (seq !== reloadSeqRef.current) return
      const msg = e instanceof Error ? e.message : String(e)
      logger.warn("chat", "useProjects", "reloadProjects failed", msg)
      setError(msg)
    } finally {
      if (seq === reloadSeqRef.current) {
        setInitialLoading(false)
        setLoading(false)
      }
    }
  }, [activeSessionIdRef])

  // Initial load.
  useEffect(() => {
    void reloadProjects()
  }, [reloadProjects])

  // Subscribe to project:* events for realtime refresh.
  useEffect(() => {
    const transport = getTransport()
    const events = [
      "project:created",
      "project:updated",
      "project:deleted",
      "project:file_uploaded",
      "project:file_deleted",
    ]
    const unsubs = events.map((name) =>
      transport.listen(name, () => {
        void reloadProjects()
      }),
    )
    return () => {
      for (const u of unsubs) u()
    }
  }, [reloadProjects])

  const createProject = useCallback(
    async (input: CreateProjectInput): Promise<Project | null> => {
      try {
        const created = await getTransport().call<Project>("create_project_cmd", {
          input,
        })
        await reloadProjects()
        return created
      } catch (e) {
        logger.warn("chat", "useProjects", "createProject failed", e)
        return null
      }
    },
    [reloadProjects],
  )

  const updateProject = useCallback(
    async (id: string, patch: UpdateProjectInput): Promise<Project | null> => {
      try {
        const updated = await getTransport().call<Project>("update_project_cmd", {
          id,
          patch,
        })
        await reloadProjects()
        return updated
      } catch (e) {
        logger.warn("chat", "useProjects", "updateProject failed", e)
        return null
      }
    },
    [reloadProjects],
  )

  const deleteProject = useCallback(
    async (id: string): Promise<boolean> => {
      try {
        const result = await getTransport().call<boolean | { deleted?: boolean }>(
          "delete_project_cmd",
          { id },
        )
        const ok =
          typeof result === "boolean" ? result : Boolean(result?.deleted ?? true)
        await reloadProjects()
        return ok
      } catch (e) {
        logger.warn("chat", "useProjects", "deleteProject failed", e)
        return false
      }
    },
    [reloadProjects],
  )

  const archiveProject = useCallback(
    async (id: string, archived: boolean): Promise<Project | null> => {
      try {
        const updated = await getTransport().call<Project>("archive_project_cmd", {
          id,
          archived,
        })
        await reloadProjects()
        return updated
      } catch (e) {
        logger.warn("chat", "useProjects", "archiveProject failed", e)
        return null
      }
    },
    [reloadProjects],
  )

  const reorderProjects = useCallback(
    async (projectIds: string[]): Promise<void> => {
      const current = projects
      const byId = new Map(current.map((project) => [project.id, project]))
      const next = [
        ...projectIds
          .map((id) => byId.get(id))
          .filter((project): project is ProjectMeta => !!project),
        ...current.filter((project) => project.archived || !projectIds.includes(project.id)),
      ]
      setProjects(next)
      try {
        await getTransport().call("reorder_projects_cmd", { projectIds })
        await reloadProjects()
      } catch (e) {
        logger.warn("chat", "useProjects", "reorderProjects failed", e)
        setProjects(current)
      }
    },
    [projects, reloadProjects],
  )

  const moveSessionToProject = useCallback(
    async (sessionId: string, projectId: string | null): Promise<void> => {
      try {
        await getTransport().call("move_session_to_project_cmd", {
          sessionId,
          projectId: projectId ?? undefined,
        })
        await reloadProjects()
      } catch (e) {
        logger.warn("chat", "useProjects", "moveSessionToProject failed", e)
      }
    },
    [reloadProjects],
  )

  return {
    projects,
    loading,
    initialLoading,
    error,
    reloadProjects,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    reorderProjects,
    moveSessionToProject,
  }
}
