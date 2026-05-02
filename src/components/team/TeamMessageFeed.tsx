import React, { useState } from "react"
import { Send } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useVirtualFeed } from "@/components/common/useVirtualFeed"
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

type TeamFeedRow =
  | { type: "empty"; key: "empty" }
  | { type: "loadMore"; key: "load-more" }
  | { type: "message"; key: string; message: TeamMessage }

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

  const rows = (() => {
    if (messages.length === 0) return [{ type: "empty", key: "empty" }]
    const next: TeamFeedRow[] = []
    if (hasMore && onLoadMore) {
      next.push({ type: "loadMore", key: "load-more" })
    }
    for (const message of messages) {
      next.push({
        type: "message",
        key: `team-message:${message.messageId}`,
        message,
      })
    }
    return next
  })() as TeamFeedRow[]

  const getRowKey = (row: TeamFeedRow) => row.key
  const estimateSize = (index: number) => {
    const row = rows[index]
    if (!row) return 56
    if (row.type === "empty") return 160
    if (row.type === "loadMore") return 32
    if (row.message.messageType === "system") return 28
    return 56
  }

  const canAnchorRow = (row: TeamFeedRow) => row.type === "message"

  const lastMessage = messages[messages.length - 1]
  const followKey = lastMessage
    ? `team-latest:${lastMessage.messageId}:${lastMessage.content.length}`
    : null
  const { scrollRef, virtualizer, virtualItems, totalSize } = useVirtualFeed({
    rows,
    getRowKey,
    estimateSize,
    overscan: 8,
    gap: 2,
    paddingStart: 8,
    paddingEnd: 8,
    followKey,
    resetKey: lastMessage?.teamId ?? "team-feed",
    canAnchorRow,
    onStartReached: onLoadMore,
    canLoadMore: hasMore,
    loadingMore,
    startThreshold: 240,
  })

  const handleSend = () => {
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
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const renderRow = (row: TeamFeedRow) => {
    if (row.type === "empty") {
      return (
        <div className="flex min-h-[40vh] items-center justify-center text-sm text-muted-foreground/50">
          {t("team.noMessages", "No messages yet")}
        </div>
      )
    }

    if (row.type === "loadMore") {
      return <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
    }

    return <TeamMessageBubble message={row.message} members={members} />
  }

  return (
    <div className="flex flex-col h-full">
      <div ref={scrollRef} className="flex-1 overflow-y-auto min-h-0">
        <div className="relative w-full" style={{ height: totalSize }}>
          {virtualItems.map((virtualRow) => {
            const row = rows[virtualRow.index]
            if (!row) return null
            return (
              <div
                key={virtualRow.key}
                ref={virtualizer.measureElement}
                data-index={virtualRow.index}
                className="absolute left-0 top-0 w-full"
                style={{ transform: `translateY(${virtualRow.start}px)` }}
              >
                {renderRow(row)}
              </div>
            )
          })}
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
