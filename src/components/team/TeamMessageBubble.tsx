import { memo, useMemo } from "react"
import { cn } from "@/lib/utils"
import type { TeamMessage, TeamMember } from "./teamTypes"

interface TeamMessageBubbleProps {
  message: TeamMessage
  members: TeamMember[]
}

function TeamMessageBubbleImpl({ message, members }: TeamMessageBubbleProps) {
  const sender = useMemo(
    () => members.find((m) => m.memberId === message.fromMemberId),
    [members, message.fromMemberId],
  )

  const timeStr = useMemo(() => {
    const d = new Date(message.timestamp)
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    })
  }, [message.timestamp])

  const isSystem = message.messageType === "system"

  if (isSystem) {
    return (
      <div className="flex justify-center px-4 py-1">
        <span className="text-[11px] italic text-muted-foreground/70">
          {message.content}
        </span>
      </div>
    )
  }

  const senderName = sender?.name ?? message.fromMemberId
  const senderColor = sender?.color ?? "#888"

  return (
    <div className="flex flex-col gap-0.5 px-3 py-1">
      {/* Header: name + time */}
      <div className="flex items-center gap-2">
        <span
          className="inline-block h-2 w-2 rounded-full shrink-0"
          style={{ backgroundColor: senderColor }}
        />
        <span
          className={cn("text-xs font-medium")}
          style={{ color: senderColor }}
        >
          {senderName}
        </span>
        {message.toMemberId && (
          <span className="text-[10px] text-muted-foreground">
            @{members.find((m) => m.memberId === message.toMemberId)?.name ?? message.toMemberId}
          </span>
        )}
        <span className="ml-auto text-[10px] tabular-nums text-muted-foreground/60">
          {timeStr}
        </span>
      </div>

      {/* Content */}
      <div className="pl-4 text-sm text-foreground leading-relaxed">
        {message.content}
      </div>
    </div>
  )
}

export const TeamMessageBubble = memo(TeamMessageBubbleImpl)
