import { useEffect, useMemo, useState } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { ArrowLeft, Check, Loader2, Plus, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type { AgentSummary } from "@/components/settings/types"
import type { TeamTemplate, TeamTemplateMember } from "@/components/team/teamTypes"
import MemberRow from "./MemberRow"

function makeBlankMember(index: number): TeamTemplateMember {
  const palette = [
    "#3B82F6",
    "#10B981",
    "#F59E0B",
    "#EF4444",
    "#8B5CF6",
    "#EC4899",
    "#06B6D4",
    "#F97316",
  ]
  return {
    name: "",
    role: "worker",
    agentId: "default",
    color: palette[index % palette.length],
    description: "",
  }
}

function slugify(name: string): string {
  return name
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48)
}

interface TemplateEditViewProps {
  templateId: string | "__new__"
  onBack: () => void
}

export default function TemplateEditView({ templateId, onBack }: TemplateEditViewProps) {
  const { t } = useTranslation()
  const isNew = templateId === "__new__"

  const [loading, setLoading] = useState(!isNew)
  const [template, setTemplate] = useState<TeamTemplate>(() => ({
    templateId: "",
    name: "",
    description: "",
    members: [makeBlankMember(0)],
  }))
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [error, setError] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [agents, setAgents] = useState<AgentSummary[]>([])
  const [agentsLoading, setAgentsLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const fetchAgents = async () => {
      try {
        const list = (await getTransport().call("list_agents", {})) as AgentSummary[]
        if (!cancelled) setAgents(list)
      } catch (e) {
        logger.error("settings", "TemplateEditView", "Failed to load agents", e)
      } finally {
        if (!cancelled) setAgentsLoading(false)
      }
    }
    fetchAgents()
    const onChanged = () => {
      if (!cancelled) fetchAgents()
    }
    window.addEventListener("agents-changed", onChanged)
    return () => {
      cancelled = true
      window.removeEventListener("agents-changed", onChanged)
    }
  }, [])

  useEffect(() => {
    if (isNew) {
      setSavedSnapshot("")
      return
    }
    let cancelled = false
    ;(async () => {
      try {
        const list = (await getTransport().call("list_team_templates", {})) as TeamTemplate[]
        const found = list.find((t) => t.templateId === templateId)
        if (!found) {
          if (!cancelled) setError(`Template "${templateId}" not found`)
          return
        }
        if (!cancelled) {
          setTemplate(found)
          setSavedSnapshot(JSON.stringify(found))
        }
      } catch (e) {
        logger.error("settings", "TemplateEditView", "Failed to load template", e)
        if (!cancelled) setError(String(e))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [templateId, isNew])

  const isDirty = useMemo(
    () => JSON.stringify(template) !== savedSnapshot,
    [template, savedSnapshot],
  )

  const updateMember = (idx: number, next: TeamTemplateMember) => {
    setTemplate((prev) => ({
      ...prev,
      members: prev.members.map((m, i) => (i === idx ? next : m)),
    }))
  }
  const addMember = () =>
    setTemplate((prev) => ({
      ...prev,
      members: [...prev.members, makeBlankMember(prev.members.length)],
    }))
  const removeMember = (idx: number) =>
    setTemplate((prev) => ({
      ...prev,
      members: prev.members.filter((_, i) => i !== idx),
    }))
  const moveMember = (idx: number, direction: -1 | 1) => {
    setTemplate((prev) => {
      const next = [...prev.members]
      const swap = idx + direction
      if (swap < 0 || swap >= next.length) return prev
      ;[next[idx], next[swap]] = [next[swap], next[idx]]
      return { ...prev, members: next }
    })
  }

  const validate = (): string | null => {
    if (!template.name.trim()) return t("settings.teamValidationName")
    if (template.members.length === 0) return t("settings.teamValidationMembers")
    for (let i = 0; i < template.members.length; i += 1) {
      const m = template.members[i]
      if (!m.name.trim()) return t("settings.teamValidationMemberName", { index: i + 1 })
      if (!m.agentId.trim()) return t("settings.teamValidationMemberAgent", { index: i + 1 })
    }
    return null
  }

  const handleSave = async () => {
    const err = validate()
    if (err) {
      setError(err)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      return
    }
    setError(null)
    setSaving(true)
    try {
      const finalTemplate: TeamTemplate = {
        ...template,
        templateId: template.templateId.trim() || slugify(template.name) || `team-${Date.now()}`,
      }
      const saved = (await getTransport().call("save_team_template", {
        template: finalTemplate,
      })) as TeamTemplate
      setTemplate(saved)
      setSavedSnapshot(JSON.stringify(saved))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "TemplateEditView", "Failed to save template", e)
      setError(String(e))
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (!template.templateId) return
    try {
      await getTransport().call("delete_team_template", {
        templateId: template.templateId,
      })
      toast.success(t("common.deleted"), {
        description: template.name,
      })
      onBack()
    } catch (e) {
      logger.error("settings", "TemplateEditView", "Failed to delete template", e)
      setError(String(e))
      toast.error(t("common.deleteFailed"), {
        description: template.name,
      })
    }
  }

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
      <div className="w-full max-w-4xl mx-auto">
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="gap-1.5 text-muted-foreground hover:text-foreground mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          <span>{t("settings.teams")}</span>
        </Button>

        <div className="flex items-center justify-between mb-5">
          <h2 className="text-lg font-semibold text-foreground">
            {isNew ? t("settings.teamNewTemplate") : template.name || t("settings.teams")}
          </h2>
          {!isNew && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setConfirmDelete(true)}
              className="text-red-500 hover:text-red-600 hover:bg-red-500/10"
            >
              <Trash2 className="h-3.5 w-3.5 mr-1" />
              {t("settings.teamDelete")}
            </Button>
          )}
        </div>

        {error && (
          <div className="mb-4 p-2.5 rounded-md border border-red-500/30 bg-red-500/5 text-xs text-red-600">
            {error}
          </div>
        )}

        <div className="space-y-5">
          <div className="space-y-4 rounded-lg border border-border bg-secondary/10 p-4">
            <div>
              <Label className="text-xs font-medium text-muted-foreground">
                {t("settings.teamTemplateName")}
              </Label>
              <Input
                className="mt-1.5 bg-background"
                value={template.name}
                onChange={(e) => setTemplate((prev) => ({ ...prev, name: e.target.value }))}
                placeholder="Full-Stack Feature"
              />
            </div>
            <div>
              <Label className="text-xs font-medium text-muted-foreground">
                {t("settings.teamTemplateId")}
              </Label>
              <Input
                className="mt-1.5 bg-background font-mono text-xs"
                value={template.templateId}
                onChange={(e) =>
                  setTemplate((prev) => ({
                    ...prev,
                    templateId: e.target.value.trim(),
                  }))
                }
                placeholder={isNew ? t("settings.teamTemplateIdAuto") : undefined}
                disabled={!isNew}
              />
            </div>
            <div>
              <Label className="text-xs font-medium text-muted-foreground">
                {t("settings.teamTemplateDesc")}
              </Label>
              <Textarea
                className="mt-1.5 min-h-[70px] bg-background text-xs"
                value={template.description}
                onChange={(e) => setTemplate((prev) => ({ ...prev, description: e.target.value }))}
                placeholder={t("settings.teamTemplateDescHint")}
              />
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between mb-3">
              <Label className="text-sm font-semibold text-foreground">
                {t("settings.teamMembers")} ({template.members.length})
              </Label>
              <Button variant="outline" size="sm" onClick={addMember}>
                <Plus className="h-3.5 w-3.5 mr-1" />
                {t("settings.teamAddMember")}
              </Button>
            </div>
            <div className="space-y-3">
              {template.members.map((m, i) => (
                <MemberRow
                  key={i}
                  index={i}
                  total={template.members.length}
                  value={m}
                  onChange={(next) => updateMember(i, next)}
                  onMoveUp={() => moveMember(i, -1)}
                  onMoveDown={() => moveMember(i, 1)}
                  onRemove={() => removeMember(i)}
                  agents={agents}
                  agentsLoading={agentsLoading}
                />
              ))}
            </div>
          </div>
        </div>

        <div className="sticky bottom-0 bg-background/95 backdrop-blur-sm -mx-6 mt-6 px-6 py-3 border-t border-border flex items-center justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onBack}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            onClick={handleSave}
            disabled={saving || !isDirty}
            className={
              saveStatus === "saved"
                ? "bg-green-500/10 text-green-600 hover:bg-green-500/15 border border-green-500/30"
                : saveStatus === "failed"
                  ? "bg-red-500/10 text-red-600 hover:bg-red-500/15 border border-red-500/30"
                  : ""
            }
          >
            {saving ? (
              <>
                <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
                {t("common.saving")}
              </>
            ) : saveStatus === "saved" ? (
              <>
                <Check className="h-3.5 w-3.5 mr-1.5" />
                {t("common.saved")}
              </>
            ) : saveStatus === "failed" ? (
              t("common.saveFailed")
            ) : (
              t("settings.teamSave")
            )}
          </Button>
        </div>
      </div>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.teamDeleteConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.teamDeleteConfirmDesc", { name: template.name })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={handleDelete} className="bg-red-500 hover:bg-red-600">
              {t("settings.teamDelete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
