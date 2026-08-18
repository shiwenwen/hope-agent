// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { useFileTabs } from "./useFileTabs"

describe("useFileTabs", () => {
  it("reveals into the active tab instead of spawning one per file", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.revealFileInActiveTab({ path: "src/a.ts", name: "a.ts" }))
    act(() => result.current.revealFileInActiveTab({ path: "src/b.ts", name: "b.ts" }))

    expect(result.current.tabs).toHaveLength(1)
    expect(result.current.tabs[0].revealFile?.path).toBe("src/b.ts")
    expect(result.current.activeId).toBe(result.current.tabs[0].id)
  })

  it("opens an explicit new tab and focuses it", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.openTab())
    const first = result.current.tabs[0].id
    act(() => result.current.openTab({ path: "src/b.ts", name: "b.ts" }))

    expect(result.current.tabs.map((tab) => tab.id)).toHaveLength(2)
    expect(result.current.activeId).not.toBe(first)
    expect(result.current.tabs[1].revealFile?.path).toBe("src/b.ts")
  })

  it("routes a reveal to the tab the user last focused", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.openTab())
    act(() => result.current.openTab())
    const [first, second] = result.current.tabs.map((tab) => tab.id)
    act(() => result.current.selectTab(first))
    act(() => result.current.revealFileInActiveTab({ path: "src/a.ts", name: "a.ts" }))

    expect(result.current.tabs[0].revealFile?.path).toBe("src/a.ts")
    expect(result.current.tabs[1].revealFile).toBeNull()
    expect(second).toBeDefined()
  })

  it("keeps the two reveal kinds mutually exclusive", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.revealFileInActiveTab({ path: "src/a.ts", name: "a.ts" }))
    act(() => result.current.revealDirectoryInActiveTab("src", null))

    expect(result.current.tabs[0].revealFile).toBeNull()
    expect(result.current.tabs[0].revealDirectory?.relPath).toBe("src")

    act(() => result.current.clearReveals())
    expect(result.current.tabs[0].revealDirectory).toBeNull()
  })

  it("falls back to a neighbouring tab when the active one closes", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.openTab())
    act(() => result.current.openTab())
    const [first, second] = result.current.tabs.map((tab) => tab.id)
    act(() => result.current.selectTab(first))
    act(() => result.current.closeTab(first))

    expect(result.current.tabs.map((tab) => tab.id)).toEqual([second])
    expect(result.current.activeId).toBe(second)
  })

  it("titles a tab from the selection its browser reports", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.openTab())
    const id = result.current.tabs[0].id
    act(() => result.current.setTabSelection(id, { name: "a.ts", relPath: "src/a.ts" }))

    expect(result.current.tabs[0].selection).toEqual({ name: "a.ts", relPath: "src/a.ts" })
  })

  it("retains a session's tabs and drops incognito ones", () => {
    const { result } = renderHook(() => useFileTabs())

    act(() => result.current.switchScope("session-a"))
    act(() => result.current.openTab({ path: "src/a.ts", name: "a.ts" }))
    act(() => result.current.switchScope("session-b"))
    expect(result.current.tabs).toHaveLength(0)

    act(() => result.current.switchScope("session-a"))
    expect(result.current.tabs[0].revealFile?.path).toBe("src/a.ts")

    // Leaving an incognito set must not retain it, and returning starts empty.
    act(() => result.current.switchScope("incognito:x", { restore: false }))
    act(() => result.current.openTab({ path: "secret.md", name: "secret.md" }))
    act(() => result.current.switchScope("session-a", { cacheCurrent: false }))
    act(() => result.current.switchScope("incognito:x", { restore: false }))
    expect(result.current.tabs).toHaveLength(0)
  })

  it("carries the draft's tabs into the session it becomes", () => {
    const { result } = renderHook(() => useFileTabs())

    // The draft scope is the initial one; a first message renames it.
    act(() => result.current.openTab({ path: "src/a.ts", name: "a.ts" }))
    act(() => result.current.renameScope("__draft__", "session-a"))

    // The open set stays put …
    expect(result.current.tabs).toHaveLength(1)
    // … and it is now addressed as the session, not as a stale draft that the
    // next new chat would inherit.
    act(() => result.current.switchScope("session-b"))
    expect(result.current.tabs).toHaveLength(0)
    act(() => result.current.switchScope("__draft__"))
    expect(result.current.tabs).toHaveLength(0)
    act(() => result.current.switchScope("session-a"))
    expect(result.current.tabs[0].revealFile?.path).toBe("src/a.ts")
  })
})
