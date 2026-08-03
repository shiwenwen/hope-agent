import { describe, expect, it, vi } from "vitest"
import type { ChatAttachment, Transport } from "@/lib/transport"
import { validateChatAttachmentCount } from "./chatPreparation"

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
