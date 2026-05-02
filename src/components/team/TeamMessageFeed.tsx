import { useCallback, useLayoutEffect, useMemo, useState } from "react"
import { Send } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Virtuoso } from "react-virtuoso"
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

const INITIAL_FIRST_ITEM_INDEX = 1_000_000

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

  // See MessageList for the tail-equal prepend detection rationale. Switching
  // teams remounts the Virtuoso (via the `key` prop below), so we don't need a
  // separate reset effect — the only state that survives is `view`, and a new
  // team's messages won't tail-match the previous team's, so the next setView
  // call lands in the non-prepend branch (firstItemIndex stays put, which is
  // fine — it's an opaque internal ID).
  const [view, setView] = useState<{ data: TeamMessage[]; firstItemIndex: number }>({
    data: messages,
    firstItemIndex: INITIAL_FIRST_ITEM_INDEX,
  })
  useLayoutEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- derived state pattern: virtuoso needs same-tick (data, firstItemIndex)
    setView((prev) => {
      if (prev.data === messages) return prev
      const delta = messages.length - prev.data.length
      const isPrepend =
        delta > 0 &&
        prev.data.length > 0 &&
        messages[messages.length - 1] === prev.data[prev.data.length - 1]
      return {
        data: messages,
        firstItemIndex: isPrepend ? prev.firstItemIndex - delta : prev.firstItemIndex,
      }
    })
  }, [messages])

  const teamId = messages[messages.length - 1]?.teamId ?? "team-feed"

  const handleStartReached = useCallback(() => {
    if (!hasMore || loadingMore || !onLoadMore) return
    void onLoadMore()
  }, [hasMore, loadingMore, onLoadMore])

  const handleSend = useCallback(() => {
    const text = draft.trim()
    if (!text) return

    // Parse @name prefix for DM
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

  const computeItemKey = useCallback(
    (_index: number, item: TeamMessage) => `team-message:${item.messageId}`,
    [],
  )

  const itemContent = useCallback(
    (_absoluteIndex: number, item: TeamMessage) => (
      <div className="pb-0.5">
        <TeamMessageBubble message={item} members={members} />
      </div>
    ),
    [members],
  )

  const Header = useCallback(
    () =>
      hasMore && onLoadMore ? (
        <div className="pt-2">
          <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
        </div>
      ) : null,
    [hasMore, loadingMore, onLoadMore],
  )

  const components = useMemo(() => ({ Header }), [Header])

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 min-h-0">
        {messages.length === 0 ? (
          <div className="flex h-full min-h-[40vh] items-center justify-center text-sm text-muted-foreground/50">
            {t("team.noMessages", "No messages yet")}
          </div>
        ) : (
          <Virtuoso
            key={teamId}
            className="h-full"
            data={view.data}
            firstItemIndex={view.firstItemIndex}
            initialTopMostItemIndex={view.data.length > 0 ? view.data.length - 1 : 0}
            computeItemKey={computeItemKey}
            itemContent={itemContent}
            components={components}
            startReached={handleStartReached}
            followOutput="auto"
            defaultItemHeight={56}
            increaseViewportBy={{ top: 100, bottom: 200 }}
            atBottomThreshold={32}
          />
        )}
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
