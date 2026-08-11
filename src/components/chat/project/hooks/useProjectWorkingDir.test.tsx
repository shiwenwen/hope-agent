// @vitest-environment jsdom

import { renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { WorkspaceAccess } from "@/lib/transport"

import { useProjectWorkingDir } from "./useProjectWorkingDir"

const access = (rootPath: string): WorkspaceAccess => ({
  readable: true,
  writeState: "enabled",
  rootPath,
})

describe("useProjectWorkingDir", () => {
  it("keeps an explicit project root without asking for the default workspace", () => {
    const transport = { getWorkspaceAccess: vi.fn() }
    const { result } = renderHook(() =>
      useProjectWorkingDir(transport, "project-1", "/workspace/explicit"),
    )

    expect(result.current).toBe("/workspace/explicit")
    expect(transport.getWorkspaceAccess).not.toHaveBeenCalled()
  })

  it("resolves a project's backend-owned default workspace", async () => {
    const transport = {
      getWorkspaceAccess: vi.fn().mockResolvedValue(access("/data/projects/project-1/workspace")),
    }
    const { result } = renderHook(() => useProjectWorkingDir(transport, "project-1", null))

    await waitFor(() => expect(result.current).toBe("/data/projects/project-1/workspace"))
    expect(transport.getWorkspaceAccess).toHaveBeenCalledWith({
      scope: "project",
      scopeId: "project-1",
    })
  })

  it("ignores a late result after switching projects", async () => {
    let resolveFirst: ((value: WorkspaceAccess) => void) | undefined
    const first = new Promise<WorkspaceAccess>((resolve) => {
      resolveFirst = resolve
    })
    const transport = {
      getWorkspaceAccess: vi
        .fn()
        .mockReturnValueOnce(first)
        .mockResolvedValueOnce(access("/data/projects/project-2/workspace")),
    }
    const { result, rerender } = renderHook(
      ({ projectId }) => useProjectWorkingDir(transport, projectId, null),
      { initialProps: { projectId: "project-1" } },
    )

    rerender({ projectId: "project-2" })
    await waitFor(() => expect(result.current).toBe("/data/projects/project-2/workspace"))
    resolveFirst?.(access("/data/projects/project-1/workspace"))
    await Promise.resolve()
    expect(result.current).toBe("/data/projects/project-2/workspace")
  })
})
