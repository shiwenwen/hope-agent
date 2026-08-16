import { describe, expect, it } from "vitest"

import { isBreadcrumbablePath, splitPathSegments } from "./filePathSegments"

describe("splitPathSegments", () => {
  it("keeps the POSIX root on the first segment", () => {
    expect(splitPathSegments("/Users/me/notes/todo.md")).toEqual([
      { label: "Users", path: "/Users", isDir: true },
      { label: "me", path: "/Users/me", isDir: true },
      { label: "notes", path: "/Users/me/notes", isDir: true },
      { label: "todo.md", path: "/Users/me/notes/todo.md", isDir: false },
    ])
  })

  it("preserves the Windows separator and keeps the drive as its own segment", () => {
    expect(splitPathSegments("C:\\work\\app\\main.rs").map((s) => s.path)).toEqual([
      "C:",
      "C:\\work",
      "C:\\work\\app",
      "C:\\work\\app\\main.rs",
    ])
  })

  it("treats a relative path as rootless and ignores trailing separators", () => {
    expect(splitPathSegments("src/lib/")).toEqual([
      { label: "src", path: "src", isDir: true },
      { label: "lib", path: "src/lib", isDir: false },
    ])
  })

  it("returns nothing for an empty or separator-only path", () => {
    expect(splitPathSegments("")).toEqual([])
    expect(splitPathSegments("///")).toEqual([])
  })
})

describe("isBreadcrumbablePath", () => {
  it("accepts multi-segment filesystem paths, Windows drives included", () => {
    expect(isBreadcrumbablePath("/Users/me/todo.md")).toBe(true)
    expect(isBreadcrumbablePath("src/lib/utils.ts")).toBe(true)
    expect(isBreadcrumbablePath("C:\\work\\main.rs")).toBe(true)
  })

  it("rejects URLs, opaque schemes and bare names", () => {
    expect(isBreadcrumbablePath("https://example.com/a/b.png")).toBe(false)
    expect(isBreadcrumbablePath("blob:http://localhost/abc")).toBe(false)
    expect(isBreadcrumbablePath("data:image/png;base64,AAAA")).toBe(false)
    expect(isBreadcrumbablePath("todo.md")).toBe(false)
  })
})
