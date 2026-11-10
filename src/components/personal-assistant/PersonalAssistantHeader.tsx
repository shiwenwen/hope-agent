import type { ComponentProps } from "react"
import { ListTodo, Search, Settings } from "lucide-react"
import { useTranslation } from "react-i18next"
import { WorkbenchHeader, WorkbenchOpenButton } from "@/components/chat/workbench/WorkbenchHeader"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { PersonalAssistantAvatar } from "./PersonalAssistantAvatar"
import type { AssistantProfile } from "./usePersonalAssistant"

export function PersonalAssistantHeader({
  profile,
  attentionCount,
  itemsOpen,
  onOpenSettings,
  onToggleItems,
  onSearch,
  workbench,
}: {
  profile: AssistantProfile | null
  attentionCount: number
  itemsOpen: boolean
  onOpenSettings: () => void
  onToggleItems: () => void
  onSearch: () => void
  workbench: ComponentProps<typeof WorkbenchHeader> & {
    docked: boolean
    onExpand: () => void
  }
}) {
  const { t } = useTranslation()
  const { docked, onExpand, ...workbenchHeader } = workbench
  const hasWorkbench = docked && workbench.tabs.length > 0
  return (
    <div
      className="flex shrink-0 items-end border-b border-border-soft bg-background"
      data-tauri-drag-region
    >
      <header
        className={cn(
          "flex h-16 min-w-0 flex-1 items-center gap-2 px-4",
          hasWorkbench && !workbench.collapsed && workbench.layoutMode === "stage" && "hidden",
        )}
        data-tauri-drag-region
      >
        <Button
          variant="ghost"
          className="h-auto min-w-0 shrink justify-start gap-3 px-1 py-0.5"
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
          {(workbench.collapsed || !hasWorkbench) && workbench.launchItems.length > 0 && (
            <WorkbenchOpenButton
              tabs={workbench.tabs}
              launchItems={workbench.launchItems}
              onOpen={onExpand}
              onOpenPanel={workbench.onOpen}
            />
          )}
        </div>
      </header>
      {hasWorkbench && <WorkbenchHeader {...workbenchHeader} />}
    </div>
  )
}
