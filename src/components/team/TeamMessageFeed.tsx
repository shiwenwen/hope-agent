import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { Send } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import LoadMoreRow from "@/components/chat/LoadMoreRow"
import type { TeamMessage, TeamMember } from "./teamTypes"
import { TeamMessageBubble } from "./TeamMessageBubble"

interface TeamMessageFeedProps {
  messages: TeamMessage[]
  members: TeamMember[]
  onSendMessage: (to: string | null, content: string) => void
  hasMore?: boolean
  loadingMore?: boolean
  onLoadMore?: () => void | Promise<void>
}

const AT_BOTTOM_THRESHOLD_PX = 32
const LOAD_MORE_THRESHOLD_PX = 200
const MAX_DOM_MESSAGES = 200
const UNLOAD_BATCH = 30

export function TeamMessageFeed({
  messages,
  members,
  onSendMessage,
  hasMore = false,
  loadingMore = false,
  onLoadMore,
}: TeamMessageFeedProps) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState("")
  const containerRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)

  const teamId = messages[messages.length - 1]?.teamId ?? "team-feed"

  const atBottomRef = useRef(true)

  // Windowed view: see MessageList for rationale.
  const [displayedStart, setDisplayedStart] = useState(0)
  const [displayedStartTeam, setDisplayedStartTeam] = useState<string>(teamId)
  if (displayedStartTeam !== teamId) {
    setDisplayedStartTeam(teamId)
    setDisplayedStart(0)
  }
  const displayedStartRef = useRef(displayedStart)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  displayedStartRef.current = displayedStart
  const messagesLenRef = useRef(messages.length)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  messagesLenRef.current = messages.length

  const items = useMemo(() => {
    const start = Math.min(displayedStart, Math.max(0, messages.length - 1))
    return messages.slice(start).map((msg, i) => ({
      msg,
      originalIndex: start + i,
    }))
  }, [messages, displayedStart])

  // Top-anchor fallback: see MessageList comment.
  const prevScrollHeightRef = useRef(0)
  const prevFirstItemMsgRef = useRef<TeamMessage | null>(items[0]?.msg ?? null)
  useLayoutEffect(() => {
    const el = containerRef.current
    if (!el) return
    const oldHeight = prevScrollHeightRef.current
    const newHeight = el.scrollHeight
    const oldFirst = prevFirstItemMsgRef.current
    const newFirst = items[0]?.msg ?? null
    if (
      newFirst &&
      oldFirst &&
      newFirst !== oldFirst &&
      newHeight > oldHeight &&
      oldHeight > 0 &&
      !atBottomRef.current
    ) {
      el.scrollTop += newHeight - oldHeight
    }
    prevScrollHeightRef.current = newHeight
    prevFirstItemMsgRef.current = newFirst
  }, [items])

  // Synchronous team-swap reset of atBottomRef.
  const lastTeamIdRef = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (lastTeamIdRef.current !== teamId) {
      lastTeamIdRef.current = teamId
      atBottomRef.current = true
    }
    const el = containerRef.current
    if (!el || !atBottomRef.current) return
    el.scrollTop = el.scrollHeight
  }, [messages, teamId])

  // ResizeObserver: watch content + container (sibling input expansion shrinks
  // the scroll container). Re-attach on teamId change.
  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return
    const el = containerRef.current
    const content = contentRef.current
    if (!el || !content) return
    const ro = new ResizeObserver(() => {
      if (atBottomRef.current) {
        el.scrollTop = el.scrollHeight
      }
    })
    ro.observe(content)
    ro.observe(el)
    return () => ro.disconnect()
  }, [teamId])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    let raf = 0
    const onScroll = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        const dist = el.scrollHeight - el.scrollTop - el.clientHeight
        const at = dist < AT_BOTTOM_THRESHOLD_PX
        atBottomRef.current = at

        // Windowed view advance: see MessageList.
        const totalLen = messagesLenRef.current
        const renderedCount = totalLen - displayedStartRef.current
        if (at && renderedCount > MAX_DOM_MESSAGES) {
          setDisplayedStart((prev) =>
            Math.min(Math.max(0, totalLen - 1), prev + UNLOAD_BATCH),
          )
        }

        if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
          if (displayedStartRef.current > 0) {
            setDisplayedStart((prev) => Math.max(0, prev - UNLOAD_BATCH))
          } else if (hasMore && !loadingMore && onLoadMore) {
            void onLoadMore()
          }
        }
      })
    }
    el.addEventListener("scroll", onScroll, { passive: true })
    return () => {
      el.removeEventListener("scroll", onScroll)
      if (raf) cancelAnimationFrame(raf)
    }
    // `teamId` belongs in deps: the outer `<div key={teamId}>` remounts the
    // scroll container on team swap, otherwise the listener stays bound to
    // the detached DOM.
  }, [teamId, hasMore, loadingMore, onLoadMore])

  const handleSend = useCallback(() => {
    const text = draft.trim()
    if (!text) return

    let to: string | null = null
    let content = text

    const atMatch = text.match(/^@(\S+)\s+(.+)$/s)
    if (atMatch) {
      const targetName = atMatch[1]
      const member = members.find((m) => m.name.toLowerCase() === targetName.toLowerCase())
      if (member) {
        to = member.memberId
        content = atMatch[2]
      }
    }

    onSendMessage(to, content)
    setDraft("")
  }, [draft, members, onSendMessage])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault()
        handleSend()
      }
    },
    [handleSend],
  )

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 min-h-0">
        <div
          ref={containerRef}
          key={teamId}
          // See MessageList for `[overflow-anchor:none]` rationale.
          className="h-full overflow-y-auto overflow-x-hidden [overflow-anchor:none]"
        >
          <div ref={contentRef}>
            {messages.length === 0 ? (
              <div className="flex h-full min-h-[40vh] items-center justify-center text-sm text-muted-foreground/50">
                {t("team.noMessages", "No messages yet")}
              </div>
            ) : (
              <>
                {hasMore && displayedStart === 0 && onLoadMore && (
                  <div className="pt-2">
                    <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
                  </div>
                )}
                {items.map(({ msg }) => (
                  <div key={`team-message:${msg.messageId}`} className="pb-0.5">
                    <TeamMessageBubble message={msg} members={members} />
                  </div>
                ))}
              </>
            )}
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-border p-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("team.messagePlaceholder", "Message team... (@name for DM)")}
          className="flex-1 h-8 text-sm"
        />
        <Button
          variant="ghost"
          size="sm"
          className="h-8 w-8 p-0 shrink-0"
          onClick={handleSend}
          disabled={!draft.trim()}
        >
          <Send className="h-4 w-4" />
        </Button>
      </div>
    </div>
  )
}
