import type {
  ForkSessionResult,
  MediaItem,
  Message,
  MessageAttachment,
  PendingFileQuote,
  PendingMessageQuote,
} from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import { createDraftAttachment, type DraftAttachment } from "@/components/chat/files/types"
import { parseUserAttachmentsMeta } from "../chatUtils"

import { isHumanAuthoredUserMessage } from "../quick-prompts/messageQuickPrompts"

export type ForkSessionRequest =
  | { sessionId: string; messageId: number }
  | { sessionId: string; beforeMessageId: number }

export interface ForkComposerDraft {
  text: string
  attachedFiles: DraftAttachment[]
  pendingQuotes: PendingFileQuote[]
  pendingMessageQuotes: PendingMessageQuote[]
}

/**
 * A finalized assistant row or a settled interrupted checkpoint is forkable.
 * Human-authored user rows are also forkable, but internal/user-shaped
 * protocol messages are not.
 */
export function isForkableConversationMessage(msg: Message): boolean {
  if (msg.role === "assistant") {
    return typeof (msg.dbId ?? msg.forkBoundaryId) === "number"
  }
  return (
    typeof msg.dbId === "number" &&
    isHumanAuthoredUserMessage(msg) &&
    (msg.content.trim().length > 0 || (msg.attachments?.length ?? 0) > 0)
  )
}

/**
 * Assistant forks include the completed reply. User forks stop immediately
 * before the selected prompt so the prompt can be edited in the new composer.
 */
export function forkSessionRequestForMessage(
  sessionId: string,
  msg: Message,
): ForkSessionRequest | null {
  if (!isForkableConversationMessage(msg)) return null
  return msg.role === "user"
    ? { sessionId, beforeMessageId: msg.dbId! }
    : { sessionId, messageId: (msg.dbId ?? msg.forkBoundaryId)! }
}

export function forkComposerTextForMessage(msg: Message): string | null {
  return msg.role === "user" ? msg.content : null
}

function mediaItemFromAttachment(attachment: MessageAttachment): MediaItem {
  return {
    url: attachment.url ?? "",
    ...(attachment.localPath ? { localPath: attachment.localPath } : {}),
    name: attachment.name,
    mimeType: attachment.mimeType,
    sizeBytes: attachment.sizeBytes,
    kind: attachment.kind === "image" ? "image" : "file",
  }
}

function parseQuoteRange(lines: string | undefined): { start: number; end: number } {
  const match = lines?.trim().match(/^(\d+)(?:-(\d+))?$/)
  if (!match) return { start: 1, end: 1 }
  const start = Math.max(1, Number(match[1]))
  const end = Math.max(start, Number(match[2] ?? match[1]))
  return { start, end }
}

function restoredQuotePath(attachment: MessageAttachment): string {
  const path = attachment.quotePath ?? ""
  const root = attachment.quoteWorktreeRoot ?? attachment.quoteProjectRoot?.path
  if (!root) return path
  const normalizedPath = path.replace(/\\/g, "/")
  const normalizedRoot = root.replace(/\\/g, "/").replace(/\/+$/, "")
  const prefix = `${normalizedRoot}/`
  return normalizedPath.startsWith(prefix) ? normalizedPath.slice(prefix.length) : path
}

async function restoreFileAttachment(attachment: MessageAttachment): Promise<DraftAttachment> {
  const lease = await getTransport().loadMediaUrl(mediaItemFromAttachment(attachment))
  try {
    const response = await fetch(lease.url)
    if (!response.ok) {
      throw new Error(`Failed to restore fork attachment: ${attachment.name}`)
    }
    const blob = await response.blob()
    const file = new File([blob], attachment.name, {
      type: attachment.mimeType || blob.type || "application/octet-stream",
    })
    return createDraftAttachment(
      file,
      "picker",
      attachment.semanticSource === "pasted_text" ? "pasted_text" : "upload",
    )
  } finally {
    lease.release()
  }
}

async function restoreFileAttachmentOrError(
  attachment: MessageAttachment,
): Promise<DraftAttachment> {
  try {
    return await restoreFileAttachment(attachment)
  } catch (error) {
    const draft = createDraftAttachment(
      new File([], attachment.name, {
        type: attachment.mimeType || "application/octet-stream",
      }),
      "picker",
      attachment.semanticSource === "pasted_text" ? "pasted_text" : "upload",
    )
    return {
      ...draft,
      status: "error",
      error:
        error instanceof Error && error.message
          ? error.message
          : `Failed to restore fork attachment: ${attachment.name}`,
    }
  }
}

async function restoreResendFileAttachment(
  attachment: MessageAttachment,
): Promise<DraftAttachment> {
  try {
    return await restoreFileAttachment(attachment)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(message.replace("fork attachment", "attachment"), { cause: error })
  }
}

function quoteDraftsFromAttachments(attachments: MessageAttachment[]): {
  pendingQuotes: PendingFileQuote[]
  pendingMessageQuotes: PendingMessageQuote[]
} {
  const pendingQuotes = attachments.flatMap((attachment): PendingFileQuote[] => {
    if (attachment.kind !== "quote" || !attachment.quotePath || attachment.quoteContent == null) {
      return []
    }
    const range = parseQuoteRange(attachment.quoteLines)
    return [
      {
        path: restoredQuotePath(attachment),
        name: attachment.name,
        startLine: range.start,
        endLine: range.end,
        content: attachment.quoteContent,
        ...(attachment.quoteProjectRoot ? { projectRoot: attachment.quoteProjectRoot } : {}),
        ...(attachment.quoteWorktreeRoot ? { worktreeRoot: attachment.quoteWorktreeRoot } : {}),
      },
    ]
  })
  const pendingMessageQuotes = attachments.flatMap((attachment): PendingMessageQuote[] =>
    attachment.kind === "message_quote" &&
    attachment.messageQuoteRole &&
    attachment.quoteContent != null
      ? [{ role: attachment.messageQuoteRole, content: attachment.quoteContent }]
      : [],
  )
  return { pendingQuotes, pendingMessageQuotes }
}

/** Rebuild the selected user prompt as a real composer draft. File metadata in
 * the fork response points at copies owned by the new session, so loading it
 * cannot make the draft depend on the source session's attachment directory. */
export async function forkComposerDraftForMessage(
  msg: Message,
  forked: ForkSessionResult,
): Promise<ForkComposerDraft | null> {
  if (msg.role !== "user") return null
  const attachments = parseUserAttachmentsMeta(forked.draftAttachmentsMeta)
  const attachedFiles = await Promise.all(
    (attachments ?? [])
      .filter(
        (attachment): attachment is MessageAttachment & { kind: "file" | "image" } =>
          attachment.kind === "file" || attachment.kind === "image",
      )
      .map(restoreFileAttachmentOrError),
  )
  const { pendingQuotes, pendingMessageQuotes } = quoteDraftsFromAttachments(attachments ?? [])
  return {
    text: msg.content,
    attachedFiles,
    pendingQuotes,
    pendingMessageQuotes,
  }
}

/** Restore the selected prompt before the source transcript is rewound.
 * Unlike fork restoration this is fail-closed: if a referenced file can no
 * longer be read, the caller must keep the original turn instead of deleting
 * it and silently resending without that attachment. */
export async function resendComposerDraftForMessage(msg: Message): Promise<ForkComposerDraft> {
  if (msg.role !== "user") throw new Error("Only user messages can be edited")
  const attachments = msg.attachments ?? []
  const attachedFiles = await Promise.all(
    attachments
      .filter(
        (attachment): attachment is MessageAttachment & { kind: "file" | "image" } =>
          attachment.kind === "file" || attachment.kind === "image",
      )
      .map(restoreResendFileAttachment),
  )
  const { pendingQuotes, pendingMessageQuotes } = quoteDraftsFromAttachments(attachments)
  return {
    text: msg.content,
    attachedFiles,
    pendingQuotes,
    pendingMessageQuotes,
  }
}
