// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import type { FileTarget } from "./types"
import { useFilePreview } from "./useFilePreview"

function workspaceTarget(name: string, nonce = 1): FileTarget {
  return {
    kind: "workspace",
    scope: "session",
    scopeId: "session-1",
    relPath: `src/${name}`,
    name,
    revealLines: { start: nonce, end: nonce, nonce },
  }
}

describe("useFilePreview", () => {
  it("deduplicates a file while refreshing its target", () => {
    const { result } = renderHook(() => useFilePreview())
    act(() => result.current.openPreview(workspaceTarget("main.ts", 1)))
    const tabId = result.current.activeId
    act(() => result.current.openPreview(workspaceTarget("main.ts", 9)))

    expect(result.current.entries).toHaveLength(1)
    expect(result.current.activeId).toBe(tabId)
    expect(result.current.target?.kind).toBe("workspace")
    if (result.current.target?.kind === "workspace") {
      expect(result.current.target.revealLines?.start).toBe(9)
    }
  })

  it("keeps multiple files, reorders them, and selects a neighbour on close", () => {
    const { result } = renderHook(() => useFilePreview())
    act(() => {
      result.current.openPreview(workspaceTarget("a.ts"))
      result.current.openPreview(workspaceTarget("b.ts"))
    })
    const [a, b] = result.current.entries
    act(() => result.current.reorderPreviews(b.id, a.id))
    expect(result.current.entries.map((entry) => entry.title)).toEqual(["b.ts", "a.ts"])

    act(() => result.current.closePreview(b.id))
    expect(result.current.activeId).toBe(a.id)
    expect(result.current.target).toEqual(a.target)
  })

  it("restores each session's preview set without cross-session rebinding", () => {
    const { result } = renderHook(() => useFilePreview())
    act(() => result.current.openPreview(workspaceTarget("one.ts")))
    act(() => result.current.switchScope("session-2"))
    expect(result.current.entries).toHaveLength(0)

    act(() => result.current.openPreview(workspaceTarget("two.ts")))
    act(() => result.current.switchScope("__draft__"))
    expect(result.current.entries.map((entry) => entry.title)).toEqual(["one.ts"])

    act(() => result.current.switchScope("session-2"))
    expect(result.current.entries.map((entry) => entry.title)).toEqual(["two.ts"])
  })

  it("burns an incognito preview set while restoring the normal draft", () => {
    const { result } = renderHook(() => useFilePreview())
    act(() => result.current.openPreview(workspaceTarget("normal.ts")))
    act(() =>
      result.current.switchScope("incognito:__draft__", {
        restore: false,
        cacheCurrent: true,
      }),
    )

    act(() => result.current.openPreview(workspaceTarget("secret.ts")))
    act(() =>
      result.current.switchScope("__draft__", {
        restore: true,
        cacheCurrent: false,
      }),
    )
    expect(result.current.entries.map((entry) => entry.title)).toEqual(["normal.ts"])

    act(() =>
      result.current.switchScope("incognito:__draft__", {
        restore: true,
        cacheCurrent: true,
      }),
    )
    expect(result.current.entries).toHaveLength(0)
  })
})
