import React, { useRef, useState } from "react"
import { X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { getTransport } from "@/lib/transport-provider"
import { useTeam } from "./useTeam"
import { TeamToolbar } from "./TeamToolbar"
import { TeamDashboard } from "./TeamDashboard"
import { TeamTaskBoard } from "./TeamTaskBoard"
import { TeamMessageFeed } from "./TeamMessageFeed"

interface TeamPanelProps {
  teamId: string
  panelWidth?: number
  onPanelWidthChange?: (w: number) => void
  onClose: () => void
  onSwitchSession?: (sessionId: string) => void
}

const MIN_WIDTH = 320
const MAX_WIDTH = 800
const DEFAULT_WIDTH = 420

export function TeamPanel({
  teamId,
  panelWidth,
  onPanelWidthChange,
  onClose,
  onSwitchSession,
}: TeamPanelProps) {
  const { t } = useTranslation()
  const { team, members, messages, tasks, sendMessage, hasMore, loadingMore, loadMoreMessages } =
    useTeam(teamId)
  const [tab, setTab] = useState("dashboard")

  // ── Drag resize handle ──────────────────────────────────
  const dragging = useRef(false)
  const startX = useRef(0)
  const startW = useRef(0)

  const width = panelWidth ?? DEFAULT_WIDTH

  const handlePointerDown = (e: React.PointerEvent) => {
    e.preventDefault()
    dragging.current = true
    startX.current = e.clientX
    startW.current = width

    const handleMove = (ev: PointerEvent) => {
      if (!dragging.current) return
      const delta = startX.current - ev.clientX // drag left = wider
      const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startW.current + delta))
      onPanelWidthChange?.(next)
    }

    const handleUp = () => {
      dragging.current = false
      document.removeEventListener("pointermove", handleMove)
      document.removeEventListener("pointerup", handleUp)
    }

    document.addEventListener("pointermove", handleMove)
    document.addEventListener("pointerup", handleUp)
  }

  // ── Actions ─────────────────────────────────────────────
  const handlePause = async () => {
    await getTransport()
      .call("pause_team", { teamId })
      .catch(() => {})
  }

  const handleResume = async () => {
    await getTransport()
      .call("resume_team", { teamId })
      .catch(() => {})
  }

  if (!team) {
    return (
      <div
        className="relative flex h-full min-h-0 shrink-0 min-w-[360px] max-w-[55%] p-3 pl-2"
        style={{ width }}
      >
        <div className="flex h-full min-h-0 w-full items-center justify-center rounded-xl border border-border/70 bg-card text-sm text-muted-foreground shadow-sm">
          {t("team.loading", "Loading...")}
        </div>
      </div>
    )
  }

  return (
    <div
      className="relative flex h-full min-h-0 shrink-0 min-w-[360px] max-w-[55%] p-3 pl-2"
      style={{ width }}
    >
      {/* Drag handle */}
      <div
        className={cn(
          "group absolute left-0 top-3 bottom-3 z-10 flex w-3 cursor-col-resize items-center justify-center",
        )}
        onPointerDown={handlePointerDown}
        role="separator"
        aria-orientation="vertical"
        aria-label={t("team.resizePanel", "Resize team panel")}
      >
        <div className="h-full w-px rounded-full bg-transparent transition-colors group-hover:bg-primary/35 group-active:bg-primary/50" />
      </div>

      <div className="relative flex h-full min-h-0 w-full flex-col overflow-hidden rounded-xl border border-border/70 bg-card shadow-sm">
        {/* Close button */}
        <Button
          variant="ghost"
          size="sm"
          className="absolute right-3 top-2.5 z-10 h-6 w-6 p-0"
          onClick={onClose}
        >
          <X className="h-3.5 w-3.5" />
        </Button>

        {/* Toolbar */}
        <TeamToolbar
          team={team}
          onPause={handlePause}
          onResume={handleResume}
          onDissolve={onClose}
        />

        {/* Tabs */}
        <Tabs value={tab} onValueChange={setTab} className="flex flex-1 flex-col min-h-0">
          <TabsList className="mx-3 mt-2">
            <TabsTrigger value="dashboard" className="flex-1 text-xs">
              {t("team.tab.dashboard", "Dashboard")}
            </TabsTrigger>
            <TabsTrigger value="tasks" className="flex-1 text-xs">
              {t("team.tab.tasks", "Tasks")}
            </TabsTrigger>
            <TabsTrigger value="messages" className="flex-1 text-xs">
              {t("team.tab.messages", "Messages")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="dashboard" className="flex-1 overflow-y-auto px-3 pb-3">
            <TeamDashboard
              members={members}
              tasks={tasks}
              team={team}
              onViewSession={onSwitchSession}
            />
          </TabsContent>

          <TabsContent value="tasks" className="flex-1 overflow-y-auto px-3 pb-3">
            <TeamTaskBoard tasks={tasks} members={members} />
          </TabsContent>

          <TabsContent value="messages" className="flex-1 min-h-0">
            <TeamMessageFeed
              messages={messages}
              members={members}
              onSendMessage={sendMessage}
              hasMore={hasMore}
              loadingMore={loadingMore}
              onLoadMore={loadMoreMessages}
            />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}
