import { describe, it, expect } from "vitest"
import { mergeArtifacts, reconcileFile } from "./useWorkspaceArtifacts"
import type { SessionFileEntry } from "./useSessionFileChanges"

interface Item {
  k: string
  src: "backend" | "live"
}
const key = (i: Item) => i.k

describe("mergeArtifacts", () => {
  it("returns the live tail as-is when backend is empty (pre-load / incognito)", () => {
    const live: Item[] = [
      { k: "a", src: "live" },
      { k: "b", src: "live" },
    ]
    expect(mergeArtifacts([], live, key)).toEqual(live)
  })

  it("puts live (most-recent window) first, backend-only older items after", () => {
    const backend: Item[] = [
      { k: "a", src: "backend" },
      { k: "b", src: "backend" },
    ]
    const live: Item[] = [{ k: "b", src: "live" }]
    const merged = mergeArtifacts(backend, live, key)
    // b was re-touched in the window → stays on top via the live tail; a
    // (older, backend-only) follows. Overlap takes the live value.
    expect(merged.map((i) => i.k)).toEqual(["b", "a"])
    expect(merged.find((i) => i.k === "b")?.src).toBe("live")
    expect(merged.find((i) => i.k === "a")?.src).toBe("backend")
  })

  it("keeps live-only entries (unpersisted streaming turn) at the top", () => {
    const backend: Item[] = [{ k: "a", src: "backend" }]
    const live: Item[] = [
      { k: "new", src: "live" },
      { k: "a", src: "live" },
    ]
    const merged = mergeArtifacts(backend, live, key)
    expect(merged.map((i) => i.k)).toEqual(["new", "a"])
    expect(merged.every((i) => i.src === "live")).toBe(true)
  })

  it("never duplicates a key present in both lists", () => {
    const backend: Item[] = [
      { k: "a", src: "backend" },
      { k: "b", src: "backend" },
    ]
    const live: Item[] = [{ k: "a", src: "live" }]
    const merged = mergeArtifacts(backend, live, key)
    expect(merged.filter((i) => i.k === "a")).toHaveLength(1)
    expect(merged).toHaveLength(2)
  })

  it("reconcile can carry a backend field onto the overlapping live entry", () => {
    interface Src {
      url: string
      origin: "web_search" | "message"
    }
    const backend: Src[] = [{ url: "u", origin: "web_search" }]
    const live: Src[] = [{ url: "u", origin: "message" }]
    const merged = mergeArtifacts(
      backend,
      live,
      (s) => s.url,
      (l, b) => (b.origin === "web_search" ? { ...l, origin: "web_search" as const } : l),
    )
    // The web_search origin (badge) is preserved even though the live tail only
    // saw a later plain-prose mention.
    expect(merged).toEqual([{ url: "u", origin: "web_search" }])
  })

  it("file reconciliation preserves backend language when a live read overlaps", () => {
    const backend: SessionFileEntry[] = [
      {
        path: "/repo/generated",
        kind: "modified",
        diff: null,
        readLines: null,
        linesAdded: 4,
        linesRemoved: 1,
        language: "typescript",
      },
    ]
    const live: SessionFileEntry[] = [
      {
        path: "/repo/generated",
        kind: "read",
        diff: null,
        readLines: 42,
        linesAdded: 0,
        linesRemoved: 0,
        language: null,
      },
    ]

    expect(mergeArtifacts(backend, live, (f) => f.path, reconcileFile)).toEqual([
      {
        path: "/repo/generated",
        kind: "read",
        diff: null,
        readLines: 42,
        linesAdded: 0,
        linesRemoved: 0,
        language: "typescript",
      },
    ])
  })
})
