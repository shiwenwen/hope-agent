/**
 * Loads and manages the project list.
 *
 * Wraps the `list_projects_cmd` / `create_project_cmd` / ... command surface
 * and transparently refreshes on EventBus `project:*` events so that any
 * mutation (from the current tab or another tab) reflows the UI within one
 * render.
 */

import { useEffect, useRef, useState, useEffectEvent } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { CreateProjectInput, Project, ProjectMeta, UpdateProjectInput } from "@/types/project"

export interface UseProjectsReturn {
  projects: ProjectMeta[]
  loading: boolean
  error: string | null
  reloadProjects: () => Promise<void>
  createProject: (input: CreateProjectInput) => Promise<Project | null>
  updateProject: (id: string, patch: UpdateProjectInput) => Promise<Project | null>
  deleteProject: (id: string) => Promise<boolean>
  archiveProject: (id: string, archived: boolean) => Promise<Project | null>
  moveSessionToProject: (sessionId: string, projectId: string | null) => Promise<void>
}

export function useProjects(options: { includeArchived?: boolean } = {}): UseProjectsReturn {
  const { includeArchived = false } = options

  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Keep the latest args in a ref so the EventBus handler always reloads
  // with the current filter without triggering reload chains.
  const includeArchivedRef = useRef(includeArchived)
  includeArchivedRef.current = includeArchived

  const reloadProjects = async () => {
    setLoading(true)
    setError(null)
    try {
      const data = await getTransport().call<ProjectMeta[]>("list_projects_cmd", {
        includeArchived: includeArchivedRef.current,
      })
      setProjects(Array.isArray(data) ? data : [])
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.warn("chat", "useProjects", "reloadProjects failed", msg)
      setError(msg)
    } finally {
      setLoading(false)
    }
  }
  const reloadProjectsEffectEvent = useEffectEvent(reloadProjects)

  // Initial load.
  useEffect(() => {
    void reloadProjectsEffectEvent()
  }, [])

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
        void reloadProjectsEffectEvent()
      }),
    )
    return () => {
      for (const u of unsubs) u()
    }
  }, [])

  const createProject = async (input: CreateProjectInput): Promise<Project | null> => {
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
  }

  const updateProject = async (id: string, patch: UpdateProjectInput): Promise<Project | null> => {
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
  }

  const deleteProject = async (id: string): Promise<boolean> => {
    try {
      const result = await getTransport().call<boolean | { deleted?: boolean }>(
        "delete_project_cmd",
        { id },
      )
      const ok = typeof result === "boolean" ? result : Boolean(result?.deleted ?? true)
      await reloadProjects()
      return ok
    } catch (e) {
      logger.warn("chat", "useProjects", "deleteProject failed", e)
      return false
    }
  }

  const archiveProject = async (id: string, archived: boolean): Promise<Project | null> => {
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
  }

  const moveSessionToProject = async (
    sessionId: string,
    projectId: string | null,
  ): Promise<void> => {
    try {
      await getTransport().call("move_session_to_project_cmd", {
        sessionId,
        projectId: projectId ?? undefined,
      })
      await reloadProjects()
    } catch (e) {
      logger.warn("chat", "useProjects", "moveSessionToProject failed", e)
    }
  }

  return {
    projects,
    loading,
    error,
    reloadProjects,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    moveSessionToProject,
  }
}
