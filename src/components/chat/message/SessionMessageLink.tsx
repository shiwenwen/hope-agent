import { memo } from "react"
import { ArrowUpRight, MessageSquare } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { requestChatFocus } from "../chatFocus"
import type { SessionMessageSource } from "@/types/chat"

interface SessionMessageLinkProps {
  direction: "sent" | "received"
  source: SessionMessageSource
  targetMessageId?: number
  className?: string
}

export default memo(function SessionMessageLink({
  direction,
  source,
  targetMessageId,
  className,
}: SessionMessageLinkProps) {
  const { t } = useTranslation()
  const title = source.title || t("chat.crossSession.untitled")
  const label =
    direction === "sent"
      ? t("chat.crossSession.sentTo", { title })
      : t("chat.crossSession.receivedFrom", { title })

  return (
    <IconTip label={label}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className={cn(
          "h-auto min-w-0 max-w-full justify-start gap-1.5 px-1.5 py-1 text-xs font-normal text-muted-foreground",
          className,
        )}
        onClick={() =>
          requestChatFocus({
            sessionId: source.sideParentSessionId || source.sessionId,
            ...(source.sideParentSessionId ? { sideSessionId: source.sessionId } : {}),
            ...(targetMessageId ? { targetMessageId } : {}),
          })
        }
      >
        <MessageSquare className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{label}</span>
        <ArrowUpRight className="h-3 w-3 shrink-0" />
      </Button>
    </IconTip>
  )
})
