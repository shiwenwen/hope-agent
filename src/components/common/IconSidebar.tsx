import { useState } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import {
  MessageSquare,
  Bot,
  Settings,
  Languages,
  Puzzle,
  CalendarDays,
  Sun,
  Moon,
  SunMoon,
  Monitor,
  User,
} from "lucide-react"
import { useTheme } from "@/hooks/useTheme"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"

interface IconSidebarProps {
  view: "chat" | "settings" | "skills" | "profile" | "agents" | "calendar"
  onOpenSettings: () => void
  onOpenChat: () => void
  onOpenAgents: () => void
  onOpenSkills: () => void
  onOpenProfile: () => void
  onOpenCalendar: () => void
  userAvatar?: string | null
  totalUnreadCount?: number
}

export default function IconSidebar({
  view,
  onOpenSettings,
  onOpenChat,
  onOpenAgents,
  onOpenSkills,
  onOpenProfile,
  onOpenCalendar,
  userAvatar,
  totalUnreadCount,
}: IconSidebarProps) {
  const { t, i18n } = useTranslation()
  const { theme, cycleTheme } = useTheme()
  const [showLangMenu, setShowLangMenu] = useState(false)

  return (
    <TooltipProvider delayDuration={100} skipDelayDuration={50}>
      <div className="w-[72px] shrink-0 border-r border-border bg-secondary/30 flex flex-col items-center">
        {/* Drag region for window movement — covers traffic light area */}
        <div className="w-full pt-10 flex flex-col items-center gap-2" data-tauri-drag-region>
          {/* User avatar (if set) */}
          {userAvatar && (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  className="w-9 h-9 rounded-full overflow-hidden ring-1 ring-primary/20 hover:ring-primary/40 transition-all cursor-pointer shrink-0"
                  onClick={onOpenProfile}
                >
                  <img
                    src={userAvatar.startsWith("/") ? convertFileSrc(userAvatar) : userAvatar}
                    className="w-full h-full object-cover"
                    alt="avatar"
                  />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("settings.profile")}</TooltipContent>
            </Tooltip>
          )}
          <div className="relative">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className={cn(
                    "rounded-xl h-8 w-8",
                    view === "chat"
                      ? "bg-primary/10 text-primary hover:bg-primary/20"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                  onClick={onOpenChat}
                >
                  <MessageSquare className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("chat.conversations")}</TooltipContent>
            </Tooltip>
            {!!totalUnreadCount && totalUnreadCount > 0 && (
              <span className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-destructive pointer-events-none" />
            )}
          </div>
        </div>

        {/* Agents entry */}
        <div className="w-full flex justify-center mt-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "agents"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={onOpenAgents}
              >
                <Bot className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("settings.agents")}</TooltipContent>
          </Tooltip>
        </div>

        {/* Skills entry */}
        <div className="w-full flex justify-center mt-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "skills"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={onOpenSkills}
              >
                <Puzzle className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("settings.skills")}</TooltipContent>
          </Tooltip>
        </div>

        {/* Calendar / Scheduled Tasks entry */}
        <div className="w-full flex justify-center mt-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "calendar"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={onOpenCalendar}
              >
                <CalendarDays className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("cron.title")}</TooltipContent>
          </Tooltip>
        </div>

        <div className="flex-1" />

        <div className="py-3 flex flex-col gap-2">
          {/* Profile */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "profile"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={onOpenProfile}
              >
                <User className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("settings.profile")}</TooltipContent>
          </Tooltip>

          {/* Theme Toggle */}
          <Tooltip>
            <TooltipTrigger asChild>
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
            </TooltipTrigger>
            <TooltipContent side="right">{`${t("theme.title")}: ${t(`theme.${theme}`)}`}</TooltipContent>
          </Tooltip>

          {/* Language Selector */}
          <div className="relative">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
                  onClick={() => setShowLangMenu(!showLangMenu)}
                >
                  <Languages className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("language.title")}</TooltipContent>
            </Tooltip>
            {showLangMenu && (
              <>
                <div className="fixed inset-0 z-40" onClick={() => setShowLangMenu(false)} />
                <div className="absolute left-12 bottom-0 z-50 bg-card border border-border rounded-lg shadow-lg py-1 min-w-[160px] max-h-[400px] overflow-y-auto">
                  {/* Follow System option */}
                  <button
                    className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                      isFollowingSystem()
                        ? "text-primary font-medium"
                        : "text-foreground"
                    }`}
                    onClick={() => {
                      setFollowSystemLanguage()
                      setShowLangMenu(false)
                    }}
                  >
                    <Monitor className="h-3.5 w-3.5 text-primary/70" />
                    <span>{t("language.system")}</span>
                    {isFollowingSystem() && (
                      <span className="ml-auto text-primary">●</span>
                    )}
                  </button>
                  <div className="border-t border-border/50 my-0.5" />
                  {SUPPORTED_LANGUAGES.map((lang) => (
                    <button
                      key={lang.code}
                      className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                        !isFollowingSystem() && (i18n.language === lang.code || (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh"))
                          ? "text-primary font-medium"
                          : "text-foreground"
                      }`}
                      onClick={() => {
                        i18n.changeLanguage(lang.code)
                        setShowLangMenu(false)
                      }}
                    >
                      <span className="text-[10px] font-bold w-5 text-primary/70">{lang.shortLabel}</span>
                      <span>{lang.label}</span>
                      {!isFollowingSystem() && (i18n.language === lang.code || (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh")) && (
                        <span className="ml-auto text-primary">●</span>
                      )}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
          {/* Settings */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "rounded-xl h-8 w-8",
                  view === "settings"
                    ? "bg-primary/10 text-primary hover:bg-primary/20"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={onOpenSettings}
              >
                <Settings className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">{t("chat.settings")}</TooltipContent>
          </Tooltip>
        </div>
      </div>
    </TooltipProvider>
  )
}
