import { useState } from "react"
import { ChevronRight } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import SessionMessageLink from "./SessionMessageLink"
import ToolCallBlock from "./ToolCallBlock"
import { getToolExecutionState } from "./executionStatus"

/** Delivery is proven by backend metadata, never by tool arguments or reply text. */
export default function SessionMessageToolResult({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const receipt = tool.metadata
  if (
    receipt?.kind !== "session_message" ||
    typeof receipt.sessionId !== "string" ||
    !receipt.sessionId.trim() ||
    !Number.isSafeInteger(receipt.messageId) ||
    receipt.messageId <= 0
  ) {
    // Some refusals are successful tool returns. Without a receipt use the
    // neutral tool name, not the generic completed label ("message sent").
    return (
      <ToolCallBlock
        tool={tool}
        labelOverride={
          getToolExecutionState(tool) === "completed" ? t(`tools.${tool.name}`) : undefined
        }
      />
    )
  }

  return (
    <div className="my-1 min-w-0 text-xs">
      <div className="flex min-w-0 items-center gap-0.5">
        <SessionMessageLink
          direction="sent"
          source={{
            sessionId: receipt.sessionId,
            title: typeof receipt.sessionTitle === "string" ? receipt.sessionTitle : undefined,
          }}
          targetMessageId={receipt.messageId}
        />
        <IconTip label={t("chat.crossSession.details")}>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0 text-muted-foreground"
            aria-label={t("chat.crossSession.details")}
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            <ChevronRight className={cn("h-3 w-3 transition-transform", expanded && "rotate-90")} />
          </Button>
        </IconTip>
      </div>
      <AnimatedCollapse open={expanded}>
        <div className="pl-5">
          <ToolCallBlock tool={tool} />
        </div>
      </AnimatedCollapse>
    </div>
  )
}
