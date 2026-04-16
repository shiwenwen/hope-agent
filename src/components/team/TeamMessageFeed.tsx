import { useState, useRef, useEffect, useCallback } from "react"
import { Send } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { TeamMessage, TeamMember } from "./teamTypes"
import { TeamMessageBubble } from "./TeamMessageBubble"

interface TeamMessageFeedProps {
  messages: TeamMessage[]
  members: TeamMember[]
  onSendMessage: (to: string | null, content: string) => void
}

export function TeamMessageFeed({
  messages,
  members,
  onSendMessage,
}: TeamMessageFeedProps) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState("")
  const scrollRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages.length])

  const handleSend = useCallback(() => {
    const text = draft.trim()
    if (!text) return

    // Parse @name prefix for DM
    let to: string | null = null
    let content = text

    const atMatch = text.match(/^@(\S+)\s+(.+)$/s)
    if (atMatch) {
      const targetName = atMatch[1]
      const member = members.find(
        (m) => m.name.toLowerCase() === targetName.toLowerCase(),
      )
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
      {/* Message list */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto min-h-0"
      >
        {messages.length === 0 ? (
          <div className="flex items-center justify-center h-full text-sm text-muted-foreground/50">
            {t("team.noMessages", "No messages yet")}
          </div>
        ) : (
          <div className="flex flex-col gap-0.5 py-2">
            {messages.map((msg) => (
              <TeamMessageBubble
                key={msg.messageId}
                message={msg}
                members={members}
              />
            ))}
            <div ref={bottomRef} />
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="flex items-center gap-2 border-t border-border p-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t(
            "team.messagePlaceholder",
            "Message team... (@name for DM)",
          )}
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
