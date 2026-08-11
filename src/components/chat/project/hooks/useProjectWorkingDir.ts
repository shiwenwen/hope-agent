import { useEffect, useState } from "react"

import { logger } from "@/lib/logger"
import type { ProjectFsScope, WorkspaceAccess } from "@/lib/transport"

interface WorkspaceAccessReader {
  getWorkspaceAccess(scope: ProjectFsScope): Promise<WorkspaceAccess>
}

interface ResolvedProjectRoot {
  projectId: string
  rootPath: string
  transport: WorkspaceAccessReader
}

/**
 * Resolve the project working directory with the same authority as backend
 * filesystem operations: explicit project root first, otherwise its canonical
 * lazily-created default workspace. Results are keyed by both project and
 * transport so a late local response cannot leak into a remote workspace.
 */
export function useProjectWorkingDir(
  transport: WorkspaceAccessReader,
  projectId: string | null,
  explicitWorkingDir: string | null,
): string | null {
  const explicit = explicitWorkingDir?.trim() ? explicitWorkingDir : null
  const [resolved, setResolved] = useState<ResolvedProjectRoot | null>(null)

  useEffect(() => {
    if (!projectId || explicit) return

    let active = true
    void transport
      .getWorkspaceAccess({ scope: "project", scopeId: projectId })
      .then((access) => {
        if (!active) return
        setResolved({ projectId, rootPath: access.rootPath, transport })
      })
      .catch(() => {
        if (!active) return
        logger.warn(
          "chat",
          "useProjectWorkingDir",
          "Failed to resolve the project's default workspace",
          { projectId },
        )
      })

    return () => {
      active = false
    }
  }, [explicit, projectId, transport])

  if (explicit) return explicit
  if (resolved?.projectId !== projectId || resolved.transport !== transport) return null
  return resolved.rootPath
}
