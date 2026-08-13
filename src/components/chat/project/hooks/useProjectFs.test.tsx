// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { WorkspaceAccess, WorkspaceListing } from "@/lib/transport"

import { useProjectFs } from "./useProjectFs"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  getWorkspaceAccess: vi.fn(),
  listen: vi.fn(() => () => {}),
  projectFsRawUrl: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => transportMock,
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

function access(rootPath: string): WorkspaceAccess {
  return { readable: true, writeState: "enabled", rootPath }
}

function listing(name: string): WorkspaceListing {
  return {
    dirRel: "",
    parentRel: null,
    entries: [
      {
        name,
        relPath: name,
        isDir: false,
        isSymlink: false,
        size: 1,
        modifiedMs: null,
      },
    ],
    truncated: false,
  }
}

beforeEach(() => {
  transportMock.call.mockReset()
  transportMock.getWorkspaceAccess.mockReset()
  transportMock.listen.mockClear()
  transportMock.projectFsRawUrl.mockReset()
})

describe("useProjectFs", () => {
  it("ignores directory and capability responses from the previous root", async () => {
    const oldAccess = deferred<WorkspaceAccess>()
    const oldListing = deferred<WorkspaceListing>()
    transportMock.getWorkspaceAccess.mockImplementation(({ scopeId }: { scopeId: string }) =>
      scopeId === "old" ? oldAccess.promise : Promise.resolve(access("/new")),
    )
    transportMock.call.mockImplementation(
      (_command: string, args: { scopeId: string }) =>
        args.scopeId === "old" ? oldListing.promise : Promise.resolve(listing("new.txt")),
    )

    const { result, rerender } = renderHook(
      ({ scopeId }) => useProjectFs("project", scopeId),
      { initialProps: { scopeId: "old" } },
    )

    let oldLoad!: Promise<void>
    act(() => {
      oldLoad = result.current.loadDir("")
    })

    rerender({ scopeId: "new" })
    await waitFor(() => expect(result.current.access?.rootPath).toBe("/new"))
    await act(async () => {
      await result.current.loadDir("")
    })
    expect(result.current.getDir("")?.entries[0]?.name).toBe("new.txt")

    await act(async () => {
      oldAccess.resolve(access("/old"))
      oldListing.resolve(listing("old.txt"))
      await oldLoad
      await Promise.resolve()
    })

    expect(result.current.access?.rootPath).toBe("/new")
    expect(result.current.getDir("")?.entries[0]?.name).toBe("new.txt")
  })
})
