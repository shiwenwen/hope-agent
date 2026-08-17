import { afterEach, describe, expect, test, vi } from "vitest"

import type { Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import type { ForkSessionResult, MediaItem, Message } from "@/types/chat"

import {
  forkComposerDraftForMessage,
  forkComposerTextForMessage,
  forkSessionRequestForMessage,
  isForkableConversationMessage,
  resendComposerDraftForMessage,
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
      loadMediaUrl: async (item: MediaItem) => ({
        url: item.url,
        release: () => undefined,
      }),
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
          revealable: false,
          project_root: { index: 1, path: "/repos/shared" },
          worktree_root: "/repos/shared-feature",
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
        revealable: false,
        projectRoot: { index: 1, path: "/repos/shared" },
        worktreeRoot: "/repos/shared-feature",
      },
    ])
    expect(draft?.pendingMessageQuotes).toEqual([{ role: "assistant", content: "quoted answer" }])
  })

  test("restores linked-worktree quote provenance when resending a prompt", async () => {
    const draft = await resendComposerDraftForMessage({
      role: "user",
      content: "revise this",
      dbId: 48,
      attachments: [
        {
          name: "brief.md",
          mimeType: "text/plain",
          sizeBytes: 0,
          kind: "quote",
          quotePath: "/repos/shared-feature/brief.md",
          quoteLines: "3-5",
          quoteContent: "quoted lines",
          quoteProjectRoot: { index: 1, path: "/repos/shared" },
          quoteWorktreeRoot: "/repos/shared-feature",
        },
      ],
    })

    expect(draft.pendingQuotes).toEqual([
      {
        path: "brief.md",
        name: "brief.md",
        startLine: 3,
        endLine: 5,
        content: "quoted lines",
        projectRoot: { index: 1, path: "/repos/shared" },
        worktreeRoot: "/repos/shared-feature",
      },
    ])
  })

  test("restores a non-revealable visual quote without inventing line one", async () => {
    const draft = await resendComposerDraftForMessage({
      role: "user",
      content: "revise this",
      dbId: 49,
      attachments: [
        {
          name: "Dashboard",
          mimeType: "text/plain",
          sizeBytes: 0,
          kind: "quote",
          quotePath: "artifact:dashboard-1",
          quoteContent: "visual selection",
          quoteRevealable: false,
        },
      ],
    })

    expect(draft.pendingQuotes).toEqual([
      {
        path: "artifact:dashboard-1",
        name: "Dashboard",
        startLine: 0,
        endLine: 0,
        content: "visual selection",
        revealable: false,
      },
    ])
  })

  test("keeps the fork draft when one copied file cannot be restored", async () => {
    setTransport({
      resolveMediaUrl: (item: MediaItem) => item.url || null,
      loadMediaUrl: async (item: MediaItem) => ({
        url: item.url,
        release: () => undefined,
      }),
    } as unknown as Transport)
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url.includes("available.png")
          ? new Response(new Blob(["image bytes"], { type: "image/png" }))
          : new Response("unavailable", { status: 503 }),
      ),
    )
    const message: Message = { role: "user", content: "revise this", dbId: 47 }
    const forked = {
      id: "fork-2",
      draftAttachmentsMeta: JSON.stringify([
        {
          name: "available.png",
          mime_type: "image/png",
          size: 11,
          url: "/api/attachments/fork-2/available.png",
        },
        {
          name: "missing.png",
          mime_type: "image/png",
          size: 12,
          url: "/api/attachments/fork-2/missing.png",
        },
      ]),
    } as ForkSessionResult

    const draft = await forkComposerDraftForMessage(message, forked)

    expect(draft?.attachedFiles).toHaveLength(2)
    expect(draft?.attachedFiles[0]).toMatchObject({ status: "ready" })
    expect(draft?.attachedFiles[1]).toMatchObject({
      status: "error",
      error: "Failed to restore fork attachment: missing.png",
    })
    expect(draft?.attachedFiles[1]?.file.name).toBe("missing.png")
  })

  test("rejects in-progress replies and internal user-shaped messages", () => {
    expect(isForkableConversationMessage({ role: "assistant", content: "streaming" })).toBe(false)
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
