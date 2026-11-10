import { ListTodo, Search, Settings } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { PersonalAssistantAvatar } from "./PersonalAssistantAvatar"
import type { AssistantProfile } from "./usePersonalAssistant"

export function PersonalAssistantHeader({
  profile,
  attentionCount,
  itemsOpen,
  onOpenSettings,
  onToggleItems,
  onSearch,
}: {
  profile: AssistantProfile | null
  attentionCount: number
  itemsOpen: boolean
  onOpenSettings: () => void
  onToggleItems: () => void
  onSearch: () => void
}) {
  const { t } = useTranslation()
  return (
    <header className="flex min-w-0 shrink-0 items-center gap-2 border-b border-border-soft px-4 py-3">
      <Button
        variant="ghost"
        className="h-auto min-w-0 justify-start gap-3 px-1 py-0.5"
        onClick={onOpenSettings}
        aria-label={t("personalAssistant.settings.title")}
      >
        <PersonalAssistantAvatar avatar={profile?.avatar} />
        <span className="min-w-0 text-left">
          <span className="block truncate text-sm font-medium">{profile?.name || "Hope"}</span>
          <span className="block text-xs font-normal text-muted-foreground">
            {t("personalAssistant.title")}
          </span>
        </span>
      </Button>
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <IconTip label={t("common.search")}>
          <Button variant="ghost" size="icon" aria-label={t("common.search")} onClick={onSearch}>
            <Search className="h-4 w-4" />
          </Button>
        </IconTip>
        <Button variant="ghost" size="sm" aria-expanded={itemsOpen} onClick={onToggleItems}>
          <ListTodo className="mr-1.5 h-4 w-4" />
          {t("personalAssistant.items")}
          {attentionCount > 0 && (
            <span className="ml-1.5 rounded-full bg-destructive px-1.5 text-xs text-white">
              {attentionCount}
            </span>
          )}
        </Button>
        <IconTip label={t("personalAssistant.settings.title")}>
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("personalAssistant.settings.title")}
            onClick={onOpenSettings}
          >
            <Settings className="h-4 w-4" />
          </Button>
        </IconTip>
      </div>
    </header>
  )
}
