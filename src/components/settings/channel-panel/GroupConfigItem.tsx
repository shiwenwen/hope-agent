import { useState } from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IconTip } from "@/components/ui/tooltip"
import { Trash2 } from "lucide-react"
import AgentAvatar from "./AgentAvatar"
import type { AgentInfo, TelegramGroupConfig } from "./types"

export default function GroupConfigItem({
  groupId,
  config,
  agents,
  onUpdate,
  onRemove,
  t,
}: {
  groupId: string
  config: TelegramGroupConfig
  agents: AgentInfo[]
  onUpdate: (patch: Partial<TelegramGroupConfig>) => void
  onRemove: () => void
  t: (key: string) => string
}) {
  const [expanded, setExpanded] = useState(false)

  const mentionLabel = groupId === "*" ? t("channels.groupIdWildcard") : groupId

  return (
    <div className="rounded-lg border bg-card p-3 space-y-2">
      {/* Header row */}
      <div className="flex items-center justify-between">
        <button
          type="button"
          className="flex items-center gap-2 text-sm font-medium hover:text-foreground transition-colors"
          onClick={() => setExpanded(!expanded)}
        >
          <span className={`transition-transform ${expanded ? "rotate-90" : ""}`}>▸</span>
          <span className="font-mono">{mentionLabel}</span>
        </button>
        <IconTip label={t("channels.removeConfig")}>
          <button
            type="button"
            className="p-1 rounded hover:bg-muted"
            onClick={onRemove}
          >
            <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
          </button>
        </IconTip>
      </div>

      {/* Compact inline controls */}
      <div className="flex items-center gap-4 flex-wrap">
        <div className="flex items-center gap-2">
          <Label className="text-xs">{t("channels.groupEnabled")}</Label>
          <Switch
            checked={config.enabled !== false}
            onCheckedChange={(v) => onUpdate({ enabled: v })}
          />
        </div>
        <div className="flex items-center gap-2">
          <Label className="text-xs">{t("channels.groupRequireMention")}</Label>
          <Select
            value={config.requireMention === null || config.requireMention === undefined ? "__inherit__" : config.requireMention ? "yes" : "no"}
            onValueChange={(v) => {
              if (v === "__inherit__") onUpdate({ requireMention: null })
              else onUpdate({ requireMention: v === "yes" })
            }}
          >
            <SelectTrigger className="h-7 text-xs w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__inherit__">{t("channels.groupRequireMentionInherit")}</SelectItem>
              <SelectItem value="yes">✓</SelectItem>
              <SelectItem value="no">✗</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex-1 min-w-[160px]">
          <Select
            value={config.agentId || "__none__"}
            onValueChange={(v) => onUpdate({ agentId: v === "__none__" ? null : v })}
          >
            <SelectTrigger className="h-7 text-xs">
              <SelectValue placeholder={t("channels.boundAgentDefault")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
              {agents.map((a) => (
                <SelectItem key={a.id} value={a.id}>
                  <span className="flex items-center gap-2">
                    <AgentAvatar agent={a} />
                    {a.name}
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Expanded details */}
      {expanded && (
        <div className="space-y-3 pt-2 border-t">
          {/* Allow from */}
          <div className="space-y-1">
            <Label className="text-xs">{t("channels.groupAllowFrom")}</Label>
            <Input
              placeholder={t("channels.groupAllowFromHint")}
              value={(config.allowFrom || []).join(", ")}
              onChange={(e) => {
                const ids = e.target.value
                  .split(/[,\n]/)
                  .map((s) => s.trim())
                  .filter(Boolean)
                onUpdate({ allowFrom: ids })
              }}
              className="text-xs h-8"
            />
          </div>

          {/* System prompt */}
          <div className="space-y-1">
            <Label className="text-xs">{t("channels.groupSystemPrompt")}</Label>
            <Input
              placeholder={t("channels.groupSystemPromptPlaceholder")}
              value={config.systemPrompt || ""}
              onChange={(e) => onUpdate({ systemPrompt: e.target.value || null })}
              className="text-xs h-8"
            />
          </div>
        </div>
      )}
    </div>
  )
}
