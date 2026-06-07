// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import { useWorkspaceEnvironment } from "./useWorkspaceEnvironment"
import type { WorkspaceEnvironmentSnapshot } from "@/lib/transport"

const transportMock = vi.hoisted(() => ({
  loadSessionEnvironment: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

function snapshot(path: string): WorkspaceEnvironmentSnapshot {
  return {
    workingDir: { path, source: "session", exists: true, name: path.split("/").pop() ?? path },
    git: null,
  }
}

describe("useWorkspaceEnvironment", () => {
  it("clears stale snapshots and refetches when the workspace scope changes", async () => {
    transportMock.loadSessionEnvironment.mockReset()
    transportMock.loadSessionEnvironment.mockResolvedValueOnce(snapshot("/old"))

    const { result, rerender } = renderHook(
      ({ refreshKey }: { refreshKey: string }) =>
        useWorkspaceEnvironment("s1", { refreshKey }),
      { initialProps: { refreshKey: "old" } },
    )

    await waitFor(() => {
      expect(result.current.snapshot?.workingDir.path).toBe("/old")
    })

    let resolveSecond: (value: WorkspaceEnvironmentSnapshot) => void = () => {}
    transportMock.loadSessionEnvironment.mockReturnValueOnce(
      new Promise<WorkspaceEnvironmentSnapshot>((resolve) => {
        resolveSecond = resolve
      }),
    )

    rerender({ refreshKey: "new" })

    await waitFor(() => {
      expect(result.current.loading).toBe(true)
      expect(result.current.snapshot).toBeNull()
    })

    await act(async () => {
      resolveSecond(snapshot("/new"))
    })

    await waitFor(() => {
      expect(result.current.snapshot?.workingDir.path).toBe("/new")
    })
    expect(transportMock.loadSessionEnvironment).toHaveBeenCalledTimes(2)
  })
})
