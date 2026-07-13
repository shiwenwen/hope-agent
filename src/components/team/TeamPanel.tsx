import { useState, useCallback } from "react"
import { X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { PANEL_SCROLL_FADE } from "../chat/right-panel/panelFade"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { getTransport } from "@/lib/transport-provider"
import { RightPanelShell } from "@/components/chat/right-panel/RightPanelShell"
import { useTeam } from "./useTeam"
import { TeamToolbar } from "./TeamToolbar"
import { TeamDashboard } from "./TeamDashboard"
import { TeamTaskBoard } from "./TeamTaskBoard"
import { TeamMessageFeed } from "./TeamMessageFeed"

interface TeamPanelProps {
  teamId: string
  panelWidth?: number
  onPanelWidthChange?: (w: number) => void
  reservedMainWidth?: number
  collapsed?: boolean
  overlay?: boolean
  animateOnMount?: boolean
  onClose: () => void
  onViewSession?: (sessionId: string) => void
}

const MIN_WIDTH = 320
const MAX_WIDTH = 800
const DEFAULT_WIDTH = 420

export function TeamPanel({
  teamId,
  panelWidth,
  onPanelWidthChange,
  reservedMainWidth,
  collapsed = false,
  overlay = false,
  animateOnMount = false,
  onClose,
  onViewSession,
}: TeamPanelProps) {
  const { t } = useTranslation()
  const { team, members, messages, tasks, sendMessage, hasMore, loadingMore, loadMoreMessages } =
    useTeam(teamId)
  const [tab, setTab] = useState("dashboard")

  const width = panelWidth ?? DEFAULT_WIDTH

  // ── Actions ─────────────────────────────────────────────
  const handlePause = useCallback(async () => {
    await getTransport()
      .call("pause_team", { teamId })
      .catch(() => {})
  }, [teamId])

  const handleResume = useCallback(async () => {
    await getTransport()
      .call("resume_team", { teamId })
      .catch(() => {})
  }, [teamId])

  if (!team) {
    return (
      <RightPanelShell
        width={width}
        onWidthChange={onPanelWidthChange}
        resizeLabel={t("team.resizePanel", "Resize team panel")}
        minWidth={MIN_WIDTH}
        maxWidth={MAX_WIDTH}
        reservedMainWidth={reservedMainWidth}
        collapsed={collapsed}
        overlay={overlay}
        animateOnMount={animateOnMount}
        contentKey="team-loading"
      >
        <div className="flex h-full min-h-0 w-full items-center justify-center text-sm text-muted-foreground">
          {t("team.loading", "Loading...")}
        </div>
      </RightPanelShell>
    )
  }

  return (
    <RightPanelShell
      width={width}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("team.resizePanel", "Resize team panel")}
      minWidth={MIN_WIDTH}
      maxWidth={MAX_WIDTH}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      overlay={overlay}
      animateOnMount={animateOnMount}
      contentKey="team"
    >
      <div className="relative flex h-full min-h-0 w-full flex-col overflow-hidden">
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

          <TabsContent
            value="dashboard"
            className={`flex-1 overflow-y-auto px-3 pb-3 ${PANEL_SCROLL_FADE}`}
          >
            <TeamDashboard
              members={members}
              tasks={tasks}
              team={team}
              onViewSession={onViewSession}
            />
          </TabsContent>

          <TabsContent
            value="tasks"
            className={`flex-1 overflow-y-auto px-3 pb-3 ${PANEL_SCROLL_FADE}`}
          >
            <TeamTaskBoard tasks={tasks} members={members} />
          </TabsContent>

          <TabsContent value="messages" className="flex-1 min-h-0">
            <TeamMessageFeed
              teamId={teamId}
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
    </RightPanelShell>
  )
}
