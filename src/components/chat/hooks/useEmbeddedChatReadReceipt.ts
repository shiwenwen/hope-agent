import { useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { Message } from "@/types/chat"
import { useReadableSurface } from "@/hooks/useReadableSurface"

/**
 * Read-watermark bridge for first-party embedded chats that stay mounted while
 * their panel is hidden.  It marks only through the newest database row that
 * MessageList has actually received, after two paint frames; opening the panel
 * can therefore never clear a reply that landed after this render snapshot.
 */
export function useEmbeddedChatReadReceipt(
  surfaceVisible: boolean,
  messageTailVisible: boolean,
  sessionId: string | null,
  messages: Message[],
): React.MutableRefObject<boolean> {
  const readable = useReadableSurface(surfaceVisible) && messageTailVisible
  const readableRef = useRef(readable)
  // Session selectors update the target id before their async transcript load
  // resolves. Keep ownership attached to the exact messages array that was
  // rendered; reusing the previous session's array must never advance the new
  // session's global database watermark. All chat stores replace the array
  // when they clear, restore a cache entry, or load the target transcript.
  const [transcriptOwner, setTranscriptOwner] = useState({ messages, sessionId })
  if (transcriptOwner.messages !== messages) {
    setTranscriptOwner({ messages, sessionId })
  }
  const transcriptMatchesSession =
    transcriptOwner.messages === messages && transcriptOwner.sessionId === sessionId
  useEffect(() => {
    readableRef.current = readable
  }, [readable])
  const renderedThroughMessageId = useMemo(
    () => messages.reduce((maximum, message) => Math.max(maximum, message.dbId ?? 0), 0),
    [messages],
  )

  useEffect(() => {
    if (!readable || !transcriptMatchesSession || !sessionId || renderedThroughMessageId <= 0) {
      return
    }
    let cancelled = false
    let firstFrame = 0
    let secondFrame = 0
    firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        if (cancelled || !readableRef.current) return
        void getTransport()
          .call("mark_session_read_cmd", {
            sessionId,
            throughMessageId: renderedThroughMessageId,
          })
          .catch(() => undefined)
      })
    })
    return () => {
      cancelled = true
      cancelAnimationFrame(firstFrame)
      cancelAnimationFrame(secondFrame)
    }
  }, [readable, renderedThroughMessageId, sessionId, transcriptMatchesSession])

  return readableRef
}
