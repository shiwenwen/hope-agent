import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { useSortable } from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ChevronDown, ChevronRight, ExternalLink, GripVertical } from "lucide-react"
import type { ProviderBadgeTone, ProviderEntry } from "./types"
import { PROVIDER_META, hasRequiredCredentials } from "./constants"
import { SearxngDockerSection } from "./SearxngDocker"

const BADGE_TONE_CLASS: Record<ProviderBadgeTone, string> = {
  positive: "bg-green-500/10 text-green-600 dark:text-green-400",
  info: "bg-blue-500/10 text-blue-600 dark:text-blue-400",
  warning: "bg-yellow-500/10 text-yellow-700 dark:text-yellow-400",
  danger: "bg-destructive/10 text-destructive",
}

export function SortableProviderItem({
  entry,
  index,
  routingRole,
  expanded,
  searxngDockerUseProxy,
  onToggleExpand,
  onToggleEnabled,
  onFieldChange,
  onSearxngDockerUseProxyChange,
  saveConfig,
}: {
  entry: ProviderEntry
  index: number
  routingRole?: "primary" | "fallback"
  expanded: boolean
  searxngDockerUseProxy: boolean
  onToggleExpand: () => void
  onToggleEnabled: (enabled: boolean) => void
  onFieldChange: (key: "apiKey" | "apiKey2" | "baseUrl", value: string | null) => void
  onSearxngDockerUseProxyChange: (enabled: boolean) => Promise<boolean>
  saveConfig: () => Promise<boolean>
}) {
  const { t } = useTranslation()
  const meta = PROVIDER_META[entry.id]
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: entry.id,
  })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  if (!meta) return null

  const canEnable = hasRequiredCredentials(entry)
  const hasFields = meta.fields.length > 0

  return (
    <div
      ref={setNodeRef}
      style={style}
      className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden"
    >
      {/* Main row */}
      <div className="flex items-center gap-2 px-3 py-2.5">
        {/* Drag handle */}
        <div
          className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
          {...attributes}
          {...listeners}
        >
          <GripVertical className="h-3.5 w-3.5" />
        </div>

        {/* Priority badge */}
        <span className="text-[10px] font-bold text-muted-foreground/50 w-5 text-center shrink-0">
          #{index + 1}
        </span>

        {/* Expand toggle + name */}
        <Button
          variant="ghost"
          className="h-auto flex-1 min-w-0 items-start justify-start gap-1.5 px-0 py-0 text-left font-normal hover:bg-transparent"
          onClick={onToggleExpand}
        >
          {hasFields ? (
            expanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            )
          ) : (
            <span className="w-3.5 shrink-0" />
          )}
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">{t(meta.labelKey)}</span>
            <span className="mt-1 flex flex-wrap gap-1">
              {meta.badges?.map((badge) => (
                <span
                  key={badge.labelKey}
                  className={`text-[10px] px-1.5 py-0.5 rounded-full font-medium ${BADGE_TONE_CLASS[badge.tone]}`}
                >
                  {t(badge.labelKey)}
                </span>
              ))}
              {!canEnable && entry.id !== "duck-duck-go" && (
                <span className="rounded-full bg-yellow-500/10 px-1.5 py-0.5 text-[10px] font-medium text-yellow-600 dark:text-yellow-400">
                  {t(
                    meta.needsApiKey
                      ? "settings.webSearchNeedsKey"
                      : "settings.webSearchNeedsConfig",
                  )}
                </span>
              )}
              {routingRole && (
                <span
                  className={
                    routingRole === "primary"
                      ? "rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary"
                      : "rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground"
                  }
                >
                  {t(
                    routingRole === "primary"
                      ? "settings.webSearchPrimary"
                      : "settings.webSearchFallback",
                  )}
                </span>
              )}
            </span>
          </span>
        </Button>

        {/* Website link */}
        <IconTip label={meta.url}>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7 shrink-0 text-muted-foreground/40 hover:text-primary"
            onClick={() => getTransport().call("open_url", { url: meta.url })}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </Button>
        </IconTip>

        {/* Enable toggle */}
        <Switch
          checked={entry.enabled}
          disabled={!canEnable && !entry.enabled}
          onCheckedChange={onToggleEnabled}
        />
      </div>

      {/* Expanded fields */}
      {expanded && hasFields && (
        <div className="px-3 pb-3 pt-1 space-y-2.5 border-t border-border/30 ml-[52px]">
          {meta.fields.map((field) => (
            <div key={field.configKey} className="space-y-1">
              <label className="text-xs font-medium text-muted-foreground">
                {t(field.labelKey)}
              </label>
              <Input
                type={field.secret ? "password" : "text"}
                placeholder={field.placeholder}
                className="h-8 text-sm"
                value={(entry[field.configKey] as string) ?? ""}
                onChange={(e) => onFieldChange(field.configKey, e.target.value || null)}
              />
            </div>
          ))}

          {/* SearXNG Docker section */}
          {entry.id === "searxng" && (
            <SearxngDockerSection
              onUrlSet={(url) => onFieldChange("baseUrl", url)}
              useProxy={searxngDockerUseProxy}
              onUseProxyChange={onSearxngDockerUseProxyChange}
              saveConfig={saveConfig}
            />
          )}
        </div>
      )}
    </div>
  )
}
