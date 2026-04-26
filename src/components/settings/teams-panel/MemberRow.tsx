import { useTranslation } from "react-i18next"
import { ArrowDown, ArrowUp, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { AgentSummary } from "@/components/settings/types"
import type { TeamTemplateMember } from "@/components/team/teamTypes"
import AgentSelector from "./AgentSelector"

const DEFAULT_COLOR_PALETTE = [
  "#3B82F6",
  "#10B981",
  "#F59E0B",
  "#EF4444",
  "#8B5CF6",
  "#EC4899",
  "#06B6D4",
  "#F97316",
]

interface MemberRowProps {
  index: number
  total: number
  value: TeamTemplateMember
  onChange: (next: TeamTemplateMember) => void
  onMoveUp: () => void
  onMoveDown: () => void
  onRemove: () => void
  agents: AgentSummary[]
  agentsLoading?: boolean
}

export default function MemberRow({
  index,
  total,
  value,
  onChange,
  onMoveUp,
  onMoveDown,
  onRemove,
  agents,
  agentsLoading,
}: MemberRowProps) {
  const { t } = useTranslation()
  const patch = (fields: Partial<TeamTemplateMember>) => onChange({ ...value, ...fields })

  return (
    <div className="rounded-lg border border-border bg-secondary/20 p-3 space-y-3">
      <div className="flex items-center gap-2">
        <span
          className="inline-block w-2 h-6 rounded-full shrink-0"
          style={{ backgroundColor: value.color || DEFAULT_COLOR_PALETTE[index % DEFAULT_COLOR_PALETTE.length] }}
        />
        <span className="text-xs text-muted-foreground font-mono">#{index + 1}</span>
        <div className="ml-auto flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={onMoveUp}
            disabled={index === 0}
          >
            <ArrowUp className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={onMoveDown}
            disabled={index === total - 1}
          >
            <ArrowDown className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 text-red-500 hover:text-red-600 hover:bg-red-500/10"
            onClick={onRemove}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberName")}
          </Label>
          <Input
            className="h-8 mt-1 bg-background"
            value={value.name}
            onChange={(e) => patch({ name: e.target.value })}
            placeholder="Frontend"
          />
        </div>
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberRole")}
          </Label>
          <Select
            value={value.role}
            onValueChange={(v) => patch({ role: v as TeamTemplateMember["role"] })}
          >
            <SelectTrigger className="h-8 mt-1 bg-background text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="worker">worker</SelectItem>
              <SelectItem value="reviewer">reviewer</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberAgent")}
          </Label>
          <div className="mt-1">
            <AgentSelector
              value={value.agentId}
              onChange={(id) => patch({ agentId: id })}
              agents={agents}
              loading={agentsLoading}
            />
          </div>
        </div>
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberColor")}
          </Label>
          <div className="flex items-center gap-1.5 mt-1">
            {DEFAULT_COLOR_PALETTE.map((c) => (
              <Button
                key={c}
                type="button"
                variant="ghost"
                size="icon"
                className={
                  "h-5 w-5 rounded-full p-0 ring-offset-1 transition-all hover:bg-transparent " +
                  (value.color === c ? "ring-2 ring-primary scale-110" : "hover:scale-110")
                }
                style={{ backgroundColor: c }}
                onClick={() => patch({ color: c })}
                aria-label={c}
              />
            ))}
          </div>
        </div>
      </div>

      <div>
        <Label className="text-[11px] text-muted-foreground">
          {t("settings.teamMemberDescription")}
        </Label>
        <div className="text-[10px] text-muted-foreground/80 mt-0.5 mb-1">
          {t("settings.teamMemberDescriptionHint")}
        </div>
        <Textarea
          className="min-h-[60px] bg-background text-xs"
          value={value.description}
          onChange={(e) => patch({ description: e.target.value })}
          placeholder="You are the frontend specialist. Build React components with TypeScript…"
        />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberModelOverride")}
          </Label>
          <Input
            className="h-8 mt-1 bg-background font-mono text-xs"
            value={value.modelOverride ?? ""}
            onChange={(e) =>
              patch({ modelOverride: e.target.value.trim() || undefined })
            }
            placeholder="provider_id/model_id"
          />
        </div>
        <div>
          <Label className="text-[11px] text-muted-foreground">
            {t("settings.teamMemberDefaultTask")}
          </Label>
          <Textarea
            className="min-h-[60px] mt-1 bg-background text-xs"
            value={value.defaultTaskTemplate ?? ""}
            onChange={(e) =>
              patch({ defaultTaskTemplate: e.target.value || undefined })
            }
            placeholder="Implement the UI for the feature."
          />
        </div>
      </div>
    </div>
  )
}
