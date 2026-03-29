import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { useSortable } from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ChevronDown, ChevronRight, ExternalLink, GripVertical } from "lucide-react"
import type { ProviderEntry } from "./types"
import { PROVIDER_META, hasRequiredCredentials } from "./constants"
import { SearxngDockerSection } from "./SearxngDocker"

export function SortableProviderItem({
  entry,
  index,
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
        <button
          className="flex items-center gap-1.5 flex-1 min-w-0 text-left"
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
          <span className="text-sm font-medium truncate">{t(meta.labelKey)}</span>
          {meta.free && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-green-500/10 text-green-600 dark:text-green-400 font-medium shrink-0">
              {t("settings.webSearchFree")}
            </span>
          )}
          {!canEnable && entry.id !== "duck-duck-go" && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-yellow-500/10 text-yellow-600 dark:text-yellow-400 font-medium shrink-0">
              {t(meta.needsApiKey ? "settings.webSearchNeedsKey" : "settings.webSearchNeedsConfig")}
            </span>
          )}
        </button>

        {/* Website link */}
        <button
          type="button"
          className="text-muted-foreground/40 hover:text-primary shrink-0 transition-colors"
          onClick={() => invoke("open_url", { url: meta.url })}
          title={meta.url}
        >
          <ExternalLink className="h-3.5 w-3.5" />
        </button>

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
