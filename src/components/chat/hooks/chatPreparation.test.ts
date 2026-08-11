import { describe, expect, it, vi } from "vitest"
import type { ChatAttachment, Transport } from "@/lib/transport"
import {
  beginChatBackendHandoff,
  composerInputDraftKey,
  deferActiveTurnRelease,
  discardChatAttachmentUploads,
  loadingStateAfterPreparationRelease,
  isUnmaterializedComposerDraftKey,
  shouldRollbackNonPersistedStoppedSend,
  validateChatAttachmentCount,
} from "./chatPreparation"

describe("composerInputDraftKey", () => {
  it("isolates lazy drafts by project until each session materializes", () => {
    expect(composerInputDraftKey(null, "project-a")).toBe("draft:project:project-a")
    expect(composerInputDraftKey(null, "project-b")).toBe("draft:project:project-b")
    expect(composerInputDraftKey(null, null)).toBe("draft:plain")
    expect(composerInputDraftKey("session-a", "project-b")).toBe("session:session-a")
    expect(isUnmaterializedComposerDraftKey("draft:project:project-a")).toBe(true)
    expect(isUnmaterializedComposerDraftKey("session:session-a")).toBe(false)
  })
})

describe("deferActiveTurnRelease", () => {
  it("keeps the turn visible through the current terminal event dispatch", async () => {
    const turns = new Map([["session", "stopped-turn"]])

    deferActiveTurnRelease(turns, "session", "stopped-turn")
    expect(turns.get("session")).toBe("stopped-turn")

    await Promise.resolve()
    expect(turns.has("session")).toBe(false)
  })

  it("does not delete a replacement turn", async () => {
    const turns = new Map([["session", "stopped-turn"]])

    deferActiveTurnRelease(turns, "session", "stopped-turn")
    turns.set("session", "replacement-turn")
    await Promise.resolve()

    expect(turns.get("session")).toBe("replacement-turn")
  })
})

describe("shouldRollbackNonPersistedStoppedSend", () => {
  it("rolls back a remote preflight Stop without a local Stop marker", () => {
    expect(shouldRollbackNonPersistedStoppedSend(false, true, false, false)).toBe(true)
  })

  it("requires a local Stop marker for ambiguous preparation and active-stream errors", () => {
    expect(shouldRollbackNonPersistedStoppedSend(false, false, true, false)).toBe(false)
    expect(shouldRollbackNonPersistedStoppedSend(false, false, false, true)).toBe(false)
    expect(shouldRollbackNonPersistedStoppedSend(true, false, true, false)).toBe(true)
    expect(shouldRollbackNonPersistedStoppedSend(true, false, false, true)).toBe(true)
  })
})

describe("discardChatAttachmentUploads", () => {
  it("does not wait for stalled cleanup after the send was stopped", async () => {
    const discardChatAttachmentUpload = vi.fn(() => new Promise<void>(() => {}))
    const transport = { discardChatAttachmentUpload } as unknown as Transport
    const attachments: ChatAttachment[] = [
      { name: "upload", mime_type: "text/plain", upload_id: "lease-1" },
    ]

    await expect(
      discardChatAttachmentUploads(attachments, transport, false),
    ).resolves.toBeUndefined()
    expect(discardChatAttachmentUpload).toHaveBeenCalledWith("lease-1")
  })

  it("waits for cleanup on ordinary failures", async () => {
    let resolveCleanup: (() => void) | undefined
    const discardChatAttachmentUpload = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveCleanup = resolve
        }),
    )
    const transport = { discardChatAttachmentUpload } as unknown as Transport
    const attachments: ChatAttachment[] = [
      { name: "upload", mime_type: "text/plain", upload_id: "lease-1" },
    ]
    let settled = false

    const cleanup = discardChatAttachmentUploads(attachments, transport, true).then(() => {
      settled = true
    })
    await Promise.resolve()
    expect(settled).toBe(false)

    resolveCleanup?.()
    await cleanup
    expect(settled).toBe(true)
  })
})

describe("validateChatAttachmentCount", () => {
  it("releases Stop immediately while excess-upload cleanup is still pending", async () => {
    const discardChatAttachmentUpload = vi.fn(() => new Promise<void>(() => {}))
    const transport = { discardChatAttachmentUpload } as unknown as Transport
    const attachments: ChatAttachment[] = Array.from({ length: 65 }, (_, index) => ({
      name: `upload-${index}`,
      mime_type: "text/plain",
      upload_id: `lease-${index}`,
    }))
    const controller = new AbortController()

    const validation = validateChatAttachmentCount(
      attachments,
      transport,
      "too many attachments",
      controller.signal,
    )
    expect(discardChatAttachmentUpload).toHaveBeenCalledTimes(65)

    controller.abort()
    await expect(validation).rejects.toMatchObject({ name: "ChatPreparationCancelledError" })
  })
})

describe("loadingStateAfterPreparationRelease", () => {
  it("does not clear a different session's active loading state", () => {
    expect(
      loadingStateAfterPreparationRelease("session-a", "session-b", new Set(["session-b"])),
    ).toBeUndefined()
  })

  it("reconciles the displayed session against active turns", () => {
    expect(loadingStateAfterPreparationRelease("session-a", "session-a", new Set())).toBe(false)
    expect(
      loadingStateAfterPreparationRelease("session-a", "session-a", new Set(["session-a"])),
    ).toBe(true)
    expect(loadingStateAfterPreparationRelease("__pending__", null, new Set())).toBe(false)
  })
})

describe("beginChatBackendHandoff", () => {
  it("does not publish a request when local Stop won the handoff", () => {
    const backendStarted = new Set<string>()

    expect(() =>
      beginChatBackendHandoff("request-a", new Set(["request-a"]), backendStarted),
    ).toThrowError("Chat preparation cancelled by user")
    expect(backendStarted.has("request-a")).toBe(false)
  })

  it("marks backend ownership synchronously when Stop has not arrived", () => {
    const backendStarted = new Set<string>()

    beginChatBackendHandoff("request-a", new Set(), backendStarted)

    expect(backendStarted.has("request-a")).toBe(true)
  })
})
