import { afterEach, describe, expect, test, vi } from "vitest"

import type { Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import type { ForkSessionResult, MediaItem, Message } from "@/types/chat"

import {
  forkComposerDraftForMessage,
  forkComposerTextForMessage,
  forkSessionRequestForMessage,
  isForkableConversationMessage,
} from "./messageFork"

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("message fork semantics", () => {
  test("includes a completed assistant reply in the fork", () => {
    const message: Message = { role: "assistant", content: "answer", dbId: 42 }

    expect(isForkableConversationMessage(message)).toBe(true)
    expect(forkSessionRequestForMessage("session-1", message)).toEqual({
      sessionId: "session-1",
      messageId: 42,
    })
    expect(forkComposerTextForMessage(message)).toBeNull()
  })

  test("includes a terminated assistant checkpoint without a final assistant row", () => {
    const message: Message = {
      role: "assistant",
      content: "",
      contentBlocks: [{ type: "text", content: "partial answer", interrupted: true }],
      forkBoundaryId: 41,
    }

    expect(isForkableConversationMessage(message)).toBe(true)
    expect(forkSessionRequestForMessage("session-1", message)).toEqual({
      sessionId: "session-1",
      messageId: 41,
    })
  })

  test("forks before a user prompt and returns it for composer editing", () => {
    const message: Message = { role: "user", content: "try another direction", dbId: 43 }

    expect(isForkableConversationMessage(message)).toBe(true)
    expect(forkSessionRequestForMessage("session-1", message)).toEqual({
      sessionId: "session-1",
      beforeMessageId: 43,
    })
    expect(forkComposerTextForMessage(message)).toBe("try another direction")
  })

  test("offers fork for an attachment-only human prompt", () => {
    const message: Message = {
      role: "user",
      content: "",
      dbId: 44,
      attachments: [
        {
          name: "reference.png",
          mimeType: "image/png",
          sizeBytes: 123,
          kind: "image",
          url: "/api/attachments/source/reference.png",
        },
      ],
    }

    expect(isForkableConversationMessage(message)).toBe(true)
    expect(forkSessionRequestForMessage("session-1", message)).toEqual({
      sessionId: "session-1",
      beforeMessageId: 44,
    })
  })

  test("restores copied files and quotes into the new composer draft", async () => {
    setTransport({
      resolveMediaUrl: (item: MediaItem) => item.url || null,
    } as unknown as Transport)
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(new Blob(["image bytes"], { type: "image/png" }))),
    )
    const message: Message = { role: "user", content: "revise this", dbId: 46 }
    const forked = {
      id: "fork-1",
      draftAttachmentsMeta: JSON.stringify([
        {
          name: "reference.png",
          mime_type: "image/png",
          size: 11,
          source: "upload",
          url: "/api/attachments/fork-1/reference.png",
        },
        {
          kind: "quote",
          name: "brief.md",
          path: "brief.md",
          lines: "3-5",
          content: "quoted lines",
        },
        {
          kind: "message_quote",
          role: "assistant",
          content: "quoted answer",
        },
      ]),
    } as ForkSessionResult

    const draft = await forkComposerDraftForMessage(message, forked)

    expect(draft?.text).toBe("revise this")
    expect(draft?.attachedFiles[0]?.file).toMatchObject({
      name: "reference.png",
      type: "image/png",
    })
    expect(draft?.pendingQuotes).toEqual([
      {
        path: "brief.md",
        name: "brief.md",
        startLine: 3,
        endLine: 5,
        content: "quoted lines",
      },
    ])
    expect(draft?.pendingMessageQuotes).toEqual([
      { role: "assistant", content: "quoted answer" },
    ])
  })

  test("rejects in-progress replies and internal user-shaped messages", () => {
    expect(
      isForkableConversationMessage({ role: "assistant", content: "streaming" }),
    ).toBe(false)
    expect(
      isForkableConversationMessage({
        role: "user",
        content: "execute plan",
        dbId: 45,
        isPlanTrigger: true,
      }),
    ).toBe(false)
  })
})
