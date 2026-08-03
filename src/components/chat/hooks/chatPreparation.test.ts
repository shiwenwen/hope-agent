import { describe, expect, it, vi } from "vitest"
import type { ChatAttachment, Transport } from "@/lib/transport"
import {
  beginChatBackendHandoff,
  loadingStateAfterPreparationRelease,
  validateChatAttachmentCount,
} from "./chatPreparation"

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
