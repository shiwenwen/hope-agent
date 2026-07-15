import { useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import ServerStatusIndicator from "@/components/common/ServerStatusIndicator"
import BrowserStatusIndicator from "@/components/common/BrowserStatusIndicator"
import type { SettingsSection } from "@/components/settings/types"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
import { useDraftSkillsStore } from "@/hooks/useDraftSkillsStore"
import { useCronUnreadStore, markAllCronRead } from "@/hooks/useCronUnreadStore"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { cn } from "@/lib/utils"
import appLogoUrl from "@/assets/logo.png"
import {
  MessageSquare,
  Bot,
  Brain,
  Settings,
  Languages,
  Puzzle,
  MessageCircle,
  CalendarDays,
  BarChart3,
  ClipboardList,
  Library,
  Server,
  Sun,
  Moon,
  SunMoon,
  Monitor,
  User,
  CheckCheck,
  ScrollText,
  PackageOpen,
} from "lucide-react"
import { useTheme } from "@/hooks/useTheme"
import {
  SUPPORTED_LANGUAGES,
  isFollowingSystem,
  setFollowSystemLanguage,
  setLanguage,
} from "@/i18n/i18n"

interface IconSidebarProps {
  view:
    | "chat"
    | "settings"
    | "skills"
    | "profile"
    | "agents"
    | "modelConfig"
    | "memory"
    | "channels"
    | "calendar"
    | "dashboard"
    | "plans"
    | "knowledge"
    | "artifacts"
  onOpenSettings: (section?: SettingsSection) => void
  onOpenChat: () => void
  onOpenAgents: () => void
  onOpenModelConfig: () => void
  onOpenChannels: () => void
  onOpenSkills: () => void
  onOpenMemory: () => void
  onOpenProfile: () => void
  onOpenCalendar: () => void
  onOpenDashboard: () => void
  onOpenPlans: () => void
  onOpenKnowledge: () => void
  onOpenArtifacts: () => void
  userAvatar?: string | null
  totalUnreadCount?: number
  onMarkAllRead?: () => void
}

export default function IconSidebar({
  view,
  onOpenSettings,
  onOpenChat,
  onOpenAgents,
  onOpenModelConfig,
  onOpenChannels,
  onOpenSkills,
  onOpenMemory,
  onOpenProfile,
  onOpenCalendar,
  onOpenDashboard,
  onOpenPlans,
  onOpenKnowledge,
  onOpenArtifacts,
  userAvatar,
  totalUnreadCount,
  onMarkAllRead,
}: IconSidebarProps) {
  const { t, i18n } = useTranslation()
  const { theme, cycleTheme } = useTheme()
  const [showLangMenu, setShowLangMenu] = useState(false)
  const { pendingUpdate } = useDesktopUpdateStore()
  const { draftCount: skillDraftCount } = useDraftSkillsStore()
  const skillDraftBadgeLabel = skillDraftCount > 99 ? "99+" : String(skillDraftCount)
  const { cronUnreadCount } = useCronUnreadStore()
  const cronUnreadBadgeLabel = cronUnreadCount > 99 ? "99+" : String(cronUnreadCount)
  const regularUnreadBadgeLabel =
    (totalUnreadCount ?? 0) > 99 ? "99+" : String(totalUnreadCount ?? 0)
  const conversationsAriaLabel =
    (totalUnreadCount ?? 0) > 0
      ? `${t("chat.conversations")}: ${t("chat.unreadConversationCount", {
          count: totalUnreadCount,
        })}`
      : t("chat.conversations")

  return (
    <div className="w-[76px] shrink-0 border-r border-border-soft bg-surface-sidebar flex flex-col items-center">
        {/* Drag region for window movement — covers traffic light area */}
        <div className="w-full pt-10 flex flex-col items-center gap-1.5" data-tauri-drag-region>
          {/* User avatar (if set) */}
          {userAvatar && (
            <IconTip label={t("settings.profileSettings")} side="right">
              <button
                className="w-9 h-9 rounded-full overflow-hidden ring-1 ring-primary/20 hover:ring-primary/40 transition-all cursor-pointer shrink-0"
                onClick={onOpenProfile}
              >
                <img
                  src={getTransport().resolveAssetUrl(userAvatar) ?? userAvatar}
                  className="w-full h-full object-cover"
                  alt="avatar"
                />
              </button>
            </IconTip>
          )}
          <ContextMenu>
            <ContextMenuTrigger asChild>
          <div className="relative">
            <IconTip label={t("chat.conversations")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "chat"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenChat}
                aria-label={conversationsAriaLabel}
              >
                <MessageSquare className="h-4 w-4" />
              </Button>
            </IconTip>
            {!!totalUnreadCount && totalUnreadCount > 0 && (
              <span
                aria-hidden="true"
                className="pointer-events-none absolute -right-1.5 -top-1 z-10 inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full border border-background bg-destructive px-1 text-[9px] font-bold leading-none text-white tabular-nums animate-in zoom-in-0 duration-200"
              >
                {regularUnreadBadgeLabel}
              </span>
            )}
          </div>
            </ContextMenuTrigger>
            <ContextMenuContent variant="floating">
              <ContextMenuItem
                onClick={async () => {
                  try {
                    await getTransport().call("mark_all_sessions_read_cmd")
                    onMarkAllRead?.()
                  } catch (err) {
                    logger.error("ui", "IconSidebar::markAllRead", "Failed to mark all as read", err)
                  }
                }}
                disabled={!totalUnreadCount || totalUnreadCount === 0}
              >
                <CheckCheck className="h-4 w-4 mr-2" />
                {t("chat.markAllRead")}
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
          {/* Knowledge Space entry — grouped directly under Conversations */}
          <div className="w-full flex justify-center">
            <IconTip label={t("knowledge.title", "Knowledge Space")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "knowledge"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenKnowledge}
              >
                <Library className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
          <div className="w-full flex justify-center">
            <IconTip label={t("artifacts.title", "Artifacts")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "artifacts"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenArtifacts}
              >
                <PackageOpen className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
          {/* Scheduled Tasks entry — grouped directly under Knowledge Space */}
          <div className="w-full flex justify-center">
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <div className="relative">
                  <IconTip label={t("cron.title")} side="right">
                    <Button
                      variant="ghost"
                      size="icon"
                      className={cn(
                        "rounded-xl h-8 w-8",
                        view === "calendar"
                          ? "bg-primary/10 text-primary hover:bg-primary/20"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                      onClick={onOpenCalendar}
                    >
                      <CalendarDays className="h-4 w-4" />
                    </Button>
                  </IconTip>
                  {cronUnreadCount > 0 && (
                    <span className="pointer-events-none absolute -right-1.5 -top-1 z-10 inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full border border-background bg-destructive px-1 text-[9px] font-bold leading-none text-white tabular-nums animate-in zoom-in-0 duration-200">
                      {cronUnreadBadgeLabel}
                    </span>
                  )}
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent variant="floating">
                <ContextMenuItem
                  disabled={cronUnreadCount === 0}
                  onSelect={() => void markAllCronRead()}
                >
                  <CheckCheck className="mr-2 h-4 w-4" />
                  {t("cron.markAllRead")}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          </div>
          {/* Dashboard / Analytics entry, grouped under Scheduled Tasks. */}
          <div className="w-full flex justify-center">
            <IconTip label={t("dashboard.title")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "dashboard"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenDashboard}
              >
                <BarChart3 className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
        </div>

        <div className="my-1 h-px w-6 bg-border-soft/80" />

        <div className="icon-sidebar-settings-shortcuts flex w-full flex-col items-center">
          {/* Agents entry */}
          <div className="w-full flex justify-center mt-1">
            <IconTip label={t("settings.agents")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "agents"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenAgents}
              >
                <Bot className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>

          {/* Model configuration entry */}
          <div className="w-full flex justify-center mt-1">
            <IconTip label={t("settings.modelConfig")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "modelConfig"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenModelConfig}
              >
                <Server className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>

          {/* Channels entry */}
          <div className="w-full flex justify-center mt-1">
            <IconTip label={t("settings.channels")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "channels"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenChannels}
              >
                <MessageCircle className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>

          {/* Skills entry */}
          <div className="w-full flex justify-center mt-1">
            <div className="relative">
              <IconTip label={t("settings.skills")} side="right">
                <Button
                  variant="ghost"
                  size="icon"
                  className={cn(
                    "rounded-xl h-8 w-8",
                    view === "skills"
                      ? "bg-primary/10 text-primary hover:bg-primary/20"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                  onClick={onOpenSkills}
                >
                  <Puzzle className="h-4 w-4" />
                </Button>
              </IconTip>
              {skillDraftCount > 0 && (
                <span className="pointer-events-none absolute -right-1.5 -top-1 z-10 inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full border border-background bg-amber-500 px-1 text-[9px] font-bold leading-none text-white tabular-nums animate-in zoom-in-0 duration-200">
                  {skillDraftBadgeLabel}
                </span>
              )}
            </div>
          </div>

          {/* Memory entry */}
          <div className="w-full flex justify-center mt-1">
            <IconTip label={t("settings.memory")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "memory"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenMemory}
              >
                <Brain className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
        </div>

        <div className="icon-sidebar-settings-shortcuts-trailing-divider my-1 h-px w-6 bg-border-soft/60" />

        {/* Browser backend — status indicator + entry to Settings → Browser.
            Green dot when a backend is live; hover shows details. */}
        <div className="w-full flex justify-center mt-1">
          <BrowserStatusIndicator onOpen={() => onOpenSettings("browser")} />
        </div>

        {/* Plans (read-only history viewer) entry */}
        <div className="w-full flex justify-center mt-1">
          <IconTip label={t("plans.title")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "plans"
                  ? "bg-primary/10 text-primary hover:bg-primary/20"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenPlans}
            >
              <ClipboardList className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>

        {/* Logs entry, quick jump to Settings -> Logs near runtime status. */}
        <div className="w-full flex justify-center mt-1">
          <IconTip label={t("settings.logs")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className="rounded-xl h-8 w-8 text-muted-foreground hover:text-foreground"
              onClick={() => onOpenSettings("logs")}
            >
              <ScrollText className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>

        <div className="flex-1" />

        <div className="py-3 flex flex-col items-center gap-2">
          {/* Server runtime health — always visible so users can catch port
              conflicts, high WS load, etc. without opening Settings. */}
          <ServerStatusIndicator onOpen={() => onOpenSettings("server")} />
          <div className="icon-sidebar-preference-shortcuts flex flex-col items-center gap-2">
            {/* Profile */}
            <IconTip label={t("settings.profileSettings")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "profile"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={onOpenProfile}
              >
                <User className="h-4 w-4" />
              </Button>
            </IconTip>

            {/* Theme Toggle */}
            <IconTip label={`${t("theme.title")}: ${t(`theme.${theme}`)}`} side="right">
              <Button
                variant="ghost"
                size="icon"
                className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
                onClick={cycleTheme}
              >
                {theme === "auto" ? (
                  <SunMoon className="h-4 w-4" />
                ) : theme === "light" ? (
                  <Sun className="h-4 w-4" />
                ) : (
                  <Moon className="h-4 w-4" />
                )}
              </Button>
            </IconTip>

            {/* Language Selector */}
            <div className="relative">
              <IconTip label={t("language.title")} side="right">
                <Button
                  variant="ghost"
                  size="icon"
                  className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
                  onClick={() => setShowLangMenu(!showLangMenu)}
                >
                  <Languages className="h-4 w-4" />
                </Button>
              </IconTip>
              {showLangMenu && (
                <div className="fixed inset-0 z-40" onClick={() => setShowLangMenu(false)} />
              )}
              <FloatingMenu
                open={showLangMenu}
                positionClassName="bottom-0 left-14"
                originClassName="origin-left"
                className="ha-menu-from-left min-w-[160px] max-h-[400px] overflow-y-auto p-1.5"
                onEscapeKeyDown={() => setShowLangMenu(false)}
              >
                    {/* Follow System option */}
                    <button
                      className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                        isFollowingSystem() ? "text-primary font-medium" : "text-foreground"
                      }`}
                      onClick={() => {
                        setFollowSystemLanguage()
                        setShowLangMenu(false)
                      }}
                    >
                      <Monitor className="h-3.5 w-3.5 text-primary/70" />
                      <span>{t("language.system")}</span>
                      {isFollowingSystem() && <span className="ml-auto text-primary">●</span>}
                    </button>
                    <div className="border-t border-border/50 my-0.5" />
                    {SUPPORTED_LANGUAGES.map((lang) => (
                      <button
                        key={lang.code}
                        className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                          !isFollowingSystem() &&
                          (i18n.language === lang.code ||
                            (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh"))
                            ? "text-primary font-medium"
                            : "text-foreground"
                        }`}
                        onClick={() => {
                          setLanguage(lang.code)
                          setShowLangMenu(false)
                        }}
                      >
                        <span className="text-[10px] font-bold w-5 text-primary/70">
                          {lang.shortLabel}
                        </span>
                        <span>{lang.label}</span>
                        {!isFollowingSystem() &&
                          (i18n.language === lang.code ||
                            (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh")) && (
                            <span className="ml-auto text-primary">●</span>
                          )}
                      </button>
                    ))}
              </FloatingMenu>
            </div>
          </div>
          {/* Settings */}
          <div className="relative flex justify-center mt-1">
            <IconTip label={t("chat.settings")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "settings"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => onOpenSettings()}
              >
                <Settings className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>

          {/* About */}
          <div className="relative flex justify-center pt-1">
            <IconTip label={t("about.title")} side="right">
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("about.title")}
                className="h-11 w-11 rounded-full border border-border-soft bg-surface-floating/80 p-0 shadow-panel hover:bg-secondary hover:shadow-floating"
                onClick={() => onOpenSettings("about")}
              >
                <span className="flex h-9 w-9 items-center justify-center overflow-hidden rounded-full bg-secondary">
                  <img src={appLogoUrl} alt="" className="h-full w-full object-cover" />
                </span>
              </Button>
            </IconTip>
            {pendingUpdate && (
              <span className="absolute -right-0.5 top-0 z-10 flex h-3 w-3">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
                <span className="relative inline-flex h-3 w-3 rounded-full border-2 border-background bg-emerald-500" />
              </span>
            )}
          </div>
        </div>
      </div>
  )
}
