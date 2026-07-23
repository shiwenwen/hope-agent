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
  CircleHelp,
  Settings,
  Languages,
  Puzzle,
  MessageCircle,
  CalendarDays,
  BarChart3,
  ClipboardList,
  Ellipsis,
  Library,
  Palette,
  Server,
  Sun,
  Moon,
  SunMoon,
  Monitor,
  CheckCheck,
  ScrollText,
  PackageOpen,
  type LucideIcon,
} from "lucide-react"
import { useTheme } from "@/hooks/useTheme"
import { openHelpWindow } from "@/lib/manual/openHelpWindow"
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
    | "design"
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
  onOpenDesign: () => void
  onOpenArtifacts: () => void
  onOpenUpdatePanel?: () => void
  userAvatar?: string | null
  totalUnreadCount?: number
  onMarkAllRead?: () => void
}

const SIDEBAR_COLLAPSE = {
  stage1Source: "[@media(max-height:880px)]:hidden",
  stage1Menu: "[@media(max-height:880px)]:flex",
  stage2Source: "[@media(max-height:840px)]:hidden",
  stage2Menu: "[@media(max-height:840px)]:flex",
  stage3Source: "[@media(max-height:800px)]:hidden",
  stage3Menu: "[@media(max-height:800px)]:flex",
  stage4Source: "[@media(max-height:760px)]:hidden",
  stage4Menu: "[@media(max-height:760px)]:flex",
  stage5Source: "[@media(max-height:720px)]:hidden",
  stage5Menu: "[@media(max-height:720px)]:flex",
  stage6Source: "[@media(max-height:680px)]:hidden",
  stage6Menu: "[@media(max-height:680px)]:flex",
} as const

export default function IconSidebar({
  view,
  onOpenSettings,
  onOpenChat,
  onOpenAgents,
  onOpenModelConfig,
  onOpenChannels,
  onOpenSkills,
  onOpenMemory,
  onOpenCalendar,
  onOpenDashboard,
  onOpenPlans,
  onOpenKnowledge,
  onOpenDesign,
  onOpenArtifacts,
  onOpenUpdatePanel,
  totalUnreadCount,
  onMarkAllRead,
}: IconSidebarProps) {
  const { t, i18n } = useTranslation()
  const { theme, cycleTheme } = useTheme()
  const [showLangMenu, setShowLangMenu] = useState(false)
  const [showMoreMenu, setShowMoreMenu] = useState(false)
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
  const collapsedItems: Array<{
    id: string
    label: string
    icon: LucideIcon
    menuClass: string
    active?: boolean
    badge?: string
    onClick: () => void
  }> = [
    {
      id: "agents",
      label: t("settings.agents"),
      icon: Bot,
      menuClass: SIDEBAR_COLLAPSE.stage5Menu,
      active: view === "agents",
      onClick: onOpenAgents,
    },
    {
      id: "modelConfig",
      label: t("settings.modelConfig"),
      icon: Server,
      menuClass: SIDEBAR_COLLAPSE.stage4Menu,
      active: view === "modelConfig",
      onClick: onOpenModelConfig,
    },
    {
      id: "channels",
      label: t("settings.channels"),
      icon: MessageCircle,
      menuClass: SIDEBAR_COLLAPSE.stage4Menu,
      active: view === "channels",
      onClick: onOpenChannels,
    },
    {
      id: "skills",
      label: t("settings.skills"),
      icon: Puzzle,
      menuClass: SIDEBAR_COLLAPSE.stage4Menu,
      active: view === "skills",
      badge: skillDraftCount > 0 ? skillDraftBadgeLabel : undefined,
      onClick: onOpenSkills,
    },
    {
      id: "memory",
      label: t("settings.memory"),
      icon: Brain,
      menuClass: SIDEBAR_COLLAPSE.stage5Menu,
      active: view === "memory",
      onClick: onOpenMemory,
    },
    {
      id: "browser",
      label: t("settings.browser.title"),
      icon: Monitor,
      menuClass: SIDEBAR_COLLAPSE.stage2Menu,
      onClick: () => onOpenSettings("browser"),
    },
    {
      id: "plans",
      label: t("plans.title"),
      icon: ClipboardList,
      menuClass: SIDEBAR_COLLAPSE.stage2Menu,
      active: view === "plans",
      onClick: onOpenPlans,
    },
    {
      id: "logs",
      label: t("settings.logs"),
      icon: ScrollText,
      menuClass: SIDEBAR_COLLAPSE.stage1Menu,
      onClick: () => onOpenSettings("logs"),
    },
    {
      id: "server",
      label: t("settings.serverRuntimeStatus"),
      icon: Server,
      menuClass: SIDEBAR_COLLAPSE.stage6Menu,
      onClick: () => onOpenSettings("server"),
    },
    {
      id: "theme",
      label: `${t("theme.title")}: ${t(`theme.${theme}`)}`,
      icon: theme === "auto" ? SunMoon : theme === "light" ? Sun : Moon,
      menuClass: SIDEBAR_COLLAPSE.stage3Menu,
      onClick: cycleTheme,
    },
    {
      id: "language",
      label: t("language.title"),
      icon: Languages,
      menuClass: SIDEBAR_COLLAPSE.stage3Menu,
      onClick: () => setShowLangMenu(true),
    },
    {
      id: "help",
      label: t("help.title"),
      icon: CircleHelp,
      menuClass: SIDEBAR_COLLAPSE.stage1Menu,
      onClick: () => void openHelpWindow(),
    },
  ]
  return (
    <div className="w-[76px] h-full min-h-0 shrink-0 overflow-hidden border-r border-border-soft bg-surface-sidebar flex flex-col items-center">
      {/* Drag region for window movement — covers traffic light area */}
      <div className="w-full pt-10 flex flex-col items-center gap-1" data-tauri-drag-region>
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
                      ? "bg-secondary text-foreground hover:bg-secondary"
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
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenKnowledge}
            >
              <Library className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
        {/* Design Space entry — grouped directly under Knowledge Space */}
        <div className="w-full flex justify-center">
          <IconTip label={t("design.title", "Design Space")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "design"
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenDesign}
            >
              <Palette className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
        {/* Artifacts entry — grouped directly under Knowledge Space */}
        <div className="w-full flex justify-center">
          <IconTip label={t("artifacts.title", "Artifacts")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "artifacts"
                  ? "bg-secondary text-foreground hover:bg-secondary"
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
                        ? "bg-secondary text-foreground hover:bg-secondary"
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
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenDashboard}
            >
              <BarChart3 className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </div>

      <div className="my-0.5 h-px w-6 bg-border-soft/80" />

      <div className="flex w-full flex-col items-center">
        {/* Agents entry */}
        <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage5Source)}>
          <IconTip label={t("settings.agents")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "agents"
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenAgents}
            >
              <Bot className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>

        {/* Model configuration entry */}
        <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage4Source)}>
          <IconTip label={t("settings.modelConfig")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "modelConfig"
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenModelConfig}
            >
              <Server className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>

        {/* Channels entry */}
        <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage4Source)}>
          <IconTip label={t("settings.channels")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "channels"
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenChannels}
            >
              <MessageCircle className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>

        {/* Skills entry */}
        <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage4Source)}>
          <div className="relative">
            <IconTip label={t("settings.skills")} side="right">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "skills"
                    ? "bg-secondary text-foreground hover:bg-secondary"
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
        <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage5Source)}>
          <IconTip label={t("settings.memory")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "memory"
                  ? "bg-secondary text-foreground hover:bg-secondary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={onOpenMemory}
            >
              <Brain className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </div>

      <div className={cn("my-0.5 h-px w-6 bg-border-soft/60", SIDEBAR_COLLAPSE.stage5Source)} />

      {/* Browser backend — status indicator + entry to Settings → Browser.
            Green dot when a backend is live; hover shows details. */}
      <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage2Source)}>
        <BrowserStatusIndicator onOpen={() => onOpenSettings("browser")} />
      </div>

      {/* Plans (read-only history viewer) entry */}
      <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage2Source)}>
        <IconTip label={t("plans.title")} side="right">
          <Button
            variant="ghost"
            size="icon"
            className={cn(
              "rounded-xl h-8 w-8",
              view === "plans"
                ? "bg-secondary text-foreground hover:bg-secondary"
                : "text-muted-foreground hover:text-foreground",
            )}
            onClick={onOpenPlans}
          >
            <ClipboardList className="h-4 w-4" />
          </Button>
        </IconTip>
      </div>

      {/* Logs entry, quick jump to Settings -> Logs near runtime status. */}
      <div className={cn("w-full flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage1Source)}>
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

      <div className={cn("hidden w-full justify-center mt-0.5", SIDEBAR_COLLAPSE.stage1Menu)}>
        <div className="relative">
          <IconTip label={t("common.more")} side="right">
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("common.more")}
              aria-expanded={showMoreMenu}
              className="rounded-xl h-8 w-8 text-muted-foreground hover:text-foreground"
              onClick={() => setShowMoreMenu((open) => !open)}
            >
              <Ellipsis className="h-4 w-4" />
            </Button>
          </IconTip>
          {showMoreMenu && (
            <div className="fixed inset-0 z-40" onClick={() => setShowMoreMenu(false)} />
          )}
          <FloatingMenu
            open={showMoreMenu}
            strategy="fixed"
            portal
            positionClassName="left-14 top-20"
            originClassName="origin-left"
            className="ha-menu-from-left w-[188px] max-h-[min(420px,calc(100vh-48px))] overflow-y-auto p-1.5"
            onEscapeKeyDown={() => setShowMoreMenu(false)}
          >
            {collapsedItems.map((item) => {
              const ItemIcon = item.icon
              return (
                <button
                  key={item.id}
                  className={cn(
                    "ha-focus-item hidden w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-colors duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground",
                    item.menuClass,
                    item.active ? "bg-secondary text-foreground" : "text-muted-foreground",
                  )}
                  onClick={() => {
                    item.onClick()
                    setShowMoreMenu(false)
                  }}
                >
                  <ItemIcon
                    className={cn(
                      "h-4 w-4 shrink-0",
                      item.active ? "text-primary" : "text-muted-foreground",
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate">{item.label}</span>
                  {item.badge && (
                    <span className="ml-auto inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full bg-amber-500 px-1 text-[9px] font-bold leading-none text-white tabular-nums">
                      {item.badge}
                    </span>
                  )}
                </button>
              )
            })}
          </FloatingMenu>
        </div>
      </div>

      <div className="flex-1" />

      <div className="py-2.5 flex shrink-0 flex-col items-center gap-1.5">
        {/* Server runtime health — always visible so users can catch port
              conflicts, high WS load, etc. without opening Settings. */}
        <div className={SIDEBAR_COLLAPSE.stage6Source}>
          <ServerStatusIndicator onOpen={() => onOpenSettings("server")} />
        </div>
        <div
          className={cn(
            "flex flex-col items-center gap-1.5",
            SIDEBAR_COLLAPSE.stage3Source,
          )}
        >
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
            <FloatingMenu
              open={showLangMenu}
              strategy="fixed"
              portal
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
        {/* Help (built-in user manual — opens its own window) */}
        <div
          className={cn("relative flex justify-center mt-0.5", SIDEBAR_COLLAPSE.stage1Source)}
        >
          <IconTip label={t("help.title")} side="right">
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("help.title")}
              className="rounded-xl h-8 w-8 text-muted-foreground hover:text-foreground"
              onClick={() => void openHelpWindow()}
            >
              <CircleHelp className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
        {/* Settings */}
        <div className="relative flex justify-center mt-0.5">
          <IconTip label={t("chat.settings")} side="right">
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "rounded-xl h-8 w-8",
                view === "settings"
                  ? "bg-secondary text-foreground hover:bg-secondary"
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
          <div className="relative flex justify-center">
            <IconTip label={t("about.title")} side="right">
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("about.title")}
                className="h-11 w-11 rounded-full border border-border-soft bg-surface-floating/80 p-0 shadow-panel hover:bg-secondary/70"
                onClick={() => onOpenSettings("about")}
              >
                <span className="flex h-9 w-9 items-center justify-center overflow-hidden rounded-full bg-secondary">
                  <img src={appLogoUrl} alt="" className="h-full w-full object-cover" />
                </span>
              </Button>
            </IconTip>
            {pendingUpdate && (
              <IconTip
                label={t("about.updateAvailable", { version: pendingUpdate.version })}
                side="right"
              >
                <Button
                  variant="outline"
                  size="sm"
                  aria-label={t("about.updateAvailable", { version: pendingUpdate.version })}
                  className="absolute bottom-[-1px] left-1/2 z-20 h-4 w-8 -translate-x-1/2 rounded-full border-0 bg-emerald-500 px-1 text-[8px] font-semibold leading-none text-white shadow-none hover:bg-emerald-600 dark:bg-emerald-500 dark:text-white dark:hover:bg-emerald-400"
                  onClick={() => {
                    if (onOpenUpdatePanel) {
                      onOpenUpdatePanel()
                    } else {
                      onOpenSettings("about")
                    }
                  }}
                >
                  <span className="truncate">{t("about.updateShort")}</span>
                </Button>
              </IconTip>
            )}
          </div>
        </div>
      </div>
      {showLangMenu && (
        <div className="fixed inset-0 z-40" onClick={() => setShowLangMenu(false)} />
      )}
    </div>
  )
}
