// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"

import {
  clearFileEditorDirty,
  confirmDiscardDirtyFileEditors,
  hasDirtyFileEditors,
  registerFileEditorDiscard,
  setFileEditorDirty,
} from "./fileDirtyRegistry"

afterEach(() => {
  for (const id of ["tab-a:file", "tab-b:file", "ownerless"]) clearFileEditorDirty(id)
  vi.restoreAllMocks()
})

describe("fileDirtyRegistry", () => {
  test("closing one owner leaves the other owner's buffer alone", () => {
    const discardA = vi.fn()
    const discardB = vi.fn()
    registerFileEditorDiscard("tab-a:file", discardA)
    registerFileEditorDiscard("tab-b:file", discardB)
    setFileEditorDirty("tab-a:file", true, "files:1")
    setFileEditorDirty("tab-b:file", true, "files:2")
    vi.spyOn(window, "confirm").mockReturnValue(true)

    expect(confirmDiscardDirtyFileEditors("discard?", "files:2")).toBe(true)

    expect(discardB).toHaveBeenCalledOnce()
    expect(discardA).not.toHaveBeenCalled()
    expect(hasDirtyFileEditors("files:1")).toBe(true)
    expect(hasDirtyFileEditors("files:2")).toBe(false)
  })

  test("a clean owner never prompts", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false)
    setFileEditorDirty("tab-a:file", true, "files:1")

    expect(confirmDiscardDirtyFileEditors("discard?", "files:2")).toBe(true)
    expect(confirmSpy).not.toHaveBeenCalled()
  })

  test("the process-wide guard still covers every editor", () => {
    const discardOwned = vi.fn()
    const discardOwnerless = vi.fn()
    registerFileEditorDiscard("tab-a:file", discardOwned)
    registerFileEditorDiscard("ownerless", discardOwnerless)
    setFileEditorDirty("tab-a:file", true, "files:1")
    setFileEditorDirty("ownerless", true)
    vi.spyOn(window, "confirm").mockReturnValue(true)

    expect(confirmDiscardDirtyFileEditors("discard?")).toBe(true)

    expect(discardOwned).toHaveBeenCalledOnce()
    expect(discardOwnerless).toHaveBeenCalledOnce()
    expect(hasDirtyFileEditors()).toBe(false)
  })

  test("declining the prompt keeps every buffer", () => {
    const discard = vi.fn()
    registerFileEditorDiscard("tab-a:file", discard)
    setFileEditorDirty("tab-a:file", true, "files:1")
    vi.spyOn(window, "confirm").mockReturnValue(false)

    expect(confirmDiscardDirtyFileEditors("discard?", "files:1")).toBe(false)
    expect(discard).not.toHaveBeenCalled()
    expect(hasDirtyFileEditors("files:1")).toBe(true)
  })
})
