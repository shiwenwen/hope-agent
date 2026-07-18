import { useCallback, useState } from "react"
import { useTranslation } from "react-i18next"
import { Bot, RefreshCw } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type { AgentSummaryForSidebar, SessionMeta } from "@/types/chat"
import SubagentSessionView from "./subagent/SubagentSessionView"

interface SubagentSessionDialogProps {
  sessionId: string | null
  agents: AgentSummaryForSidebar[]
  onOpenChange: (open: boolean) => void
  onOpenNestedSession?: (sessionId: string) => void
}

/**
 * Modal transcript viewer for an arbitrary child session. Still used for cases
 * the sub-agent panel can't select by run id — team-member sessions and skill
 * fork cards. The load/stream state machine lives in {@link SubagentSessionView};
 * this shell only owns the dialog chrome + header title.
 */
export default function SubagentSessionDialog({
  sessionId,
  agents,
  onOpenChange,
  onOpenNestedSession,
}: SubagentSessionDialogProps) {
  const { t } = useTranslation()
  const [reloadToken, setReloadToken] = useState(0)
  const [titleState, setTitleState] = useState<{ sessionId: string; title: string | null } | null>(
    null,
  )

  // Reuse the meta the embedded view already loads (no second get_session_cmd).
  const handleMeta = useCallback(
    (meta: SessionMeta | null) => {
      if (sessionId) setTitleState({ sessionId, title: meta?.title?.trim() || null })
    },
    [sessionId],
  )

  // Ignore a stale title left over from a previously-previewed session.
  const title = titleState && titleState.sessionId === sessionId ? titleState.title : null
  const heading = title || t("subagent.dialog.untitled", { defaultValue: "Sub-agent session" })
  const subtitle = sessionId
    ? t("subagent.dialog.subtitle", {
        defaultValue: "Live view · {{sessionId}}",
        sessionId: sessionId.slice(0, 8),
      })
    : ""

  return (
    <Dialog open={!!sessionId} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(86vh,900px)] max-h-[86vh] w-[min(1200px,calc(100vw-2rem))] max-w-none flex-col gap-0 overflow-hidden p-0 sm:rounded-xl">
        <DialogHeader className="border-b border-border px-4 py-3 pr-12 sm:px-5">
          <div className="flex min-w-0 items-center gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Bot className="h-4 w-4" />
            </div>
            <div className="min-w-0 flex-1 text-left">
              <DialogTitle className="truncate text-base">{heading}</DialogTitle>
              <DialogDescription className="truncate text-xs">{subtitle}</DialogDescription>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 shrink-0"
              onClick={() => setReloadToken((n) => n + 1)}
              disabled={!sessionId}
              aria-label={t("common.refresh", { defaultValue: "Refresh" })}
            >
              <RefreshCw className="h-4 w-4" />
            </Button>
          </div>
        </DialogHeader>

        <SubagentSessionView
          sessionId={sessionId}
          agents={agents}
          reloadToken={reloadToken}
          onOpenNestedSession={onOpenNestedSession}
          onMeta={handleMeta}
        />
      </DialogContent>
    </Dialog>
  )
}
