/**
 * Create / edit project dialog.
 *
 * Reused for both flows:
 *  - `mode="create"` + `initialProject=undefined` → blank form, calls onCreate
 *  - `mode="edit"` + `initialProject=<Project>` → prefilled form, calls onUpdate
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Check } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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

import type {
  CreateProjectInput,
  Project,
  UpdateProjectInput,
} from "@/types/project"
import type { AgentSummaryForSidebar } from "@/types/chat"

export interface ProjectDialogProps {
  open: boolean
  mode: "create" | "edit"
  initialProject?: Project | null
  agents: AgentSummaryForSidebar[]
  onOpenChange: (open: boolean) => void
  onCreate?: (input: CreateProjectInput) => Promise<Project | null>
  onUpdate?: (id: string, patch: UpdateProjectInput) => Promise<Project | null>
}

const COLOR_CHOICES = [
  { value: "amber", label: "amber", className: "bg-amber-500" },
  { value: "violet", label: "violet", className: "bg-violet-500" },
  { value: "sky", label: "sky", className: "bg-sky-500" },
  { value: "emerald", label: "emerald", className: "bg-emerald-500" },
  { value: "rose", label: "rose", className: "bg-rose-500" },
  { value: "indigo", label: "indigo", className: "bg-indigo-500" },
  { value: "slate", label: "slate", className: "bg-slate-500" },
]

export default function ProjectDialog({
  open,
  mode,
  initialProject,
  agents,
  onOpenChange,
  onCreate,
  onUpdate,
}: ProjectDialogProps) {
  const { t } = useTranslation()

  const [name, setName] = useState("")
  const [description, setDescription] = useState("")
  const [instructions, setInstructions] = useState("")
  const [emoji, setEmoji] = useState("")
  const [color, setColor] = useState<string>("")
  const [defaultAgentId, setDefaultAgentId] = useState<string>("")

  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">(
    "idle",
  )
  const [error, setError] = useState("")

  useEffect(() => {
    if (!open) return
    setError("")
    setSaveStatus("idle")
    if (mode === "edit" && initialProject) {
      setName(initialProject.name ?? "")
      setDescription(initialProject.description ?? "")
      setInstructions(initialProject.instructions ?? "")
      setEmoji(initialProject.emoji ?? "")
      setColor(initialProject.color ?? "")
      setDefaultAgentId(initialProject.defaultAgentId ?? "")
    } else {
      setName("")
      setDescription("")
      setInstructions("")
      setEmoji("")
      setColor("")
      setDefaultAgentId("")
    }
  }, [open, mode, initialProject])

  async function handleSave() {
    if (!name.trim()) {
      setError(t("project.projectName") + " ?")
      return
    }
    setSaving(true)
    setError("")
    try {
      if (mode === "create" && onCreate) {
        const created = await onCreate({
          name: name.trim(),
          description: description.trim() || null,
          instructions: instructions.trim() || null,
          emoji: emoji.trim() || null,
          color: color || null,
          defaultAgentId: defaultAgentId || null,
        })
        if (created) {
          setSaveStatus("saved")
          setTimeout(() => onOpenChange(false), 400)
        } else {
          setSaveStatus("failed")
        }
      } else if (mode === "edit" && initialProject && onUpdate) {
        const updated = await onUpdate(initialProject.id, {
          name: name.trim(),
          description: description.trim(),
          instructions: instructions.trim(),
          emoji: emoji.trim(),
          color: color,
          defaultAgentId: defaultAgentId,
        })
        if (updated) {
          setSaveStatus("saved")
          setTimeout(() => onOpenChange(false), 400)
        } else {
          setSaveStatus("failed")
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setSaveStatus("failed")
    } finally {
      setSaving(false)
      setTimeout(() => setSaveStatus("idle"), 2000)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {mode === "create" ? t("project.newProject") : t("project.editProject")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="grid grid-cols-6 gap-3">
            <div className="col-span-1 space-y-1.5">
              <Label htmlFor="project-emoji">{t("project.projectEmoji")}</Label>
              <Input
                id="project-emoji"
                value={emoji}
                onChange={(e) => setEmoji(e.target.value)}
                placeholder="🚀"
                maxLength={4}
                className="text-center text-lg"
              />
            </div>
            <div className="col-span-5 space-y-1.5">
              <Label htmlFor="project-name">{t("project.projectName")}</Label>
              <Input
                id="project-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("project.projectNamePlaceholder")}
                autoFocus
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="project-description">
              {t("project.projectDescription")}
            </Label>
            <Textarea
              id="project-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t("project.projectDescriptionPlaceholder")}
              rows={2}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="project-instructions">
              {t("project.projectInstructions")}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t("project.projectInstructionsHint")}
            </p>
            <Textarea
              id="project-instructions"
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              placeholder={t("project.projectInstructionsPlaceholder")}
              rows={5}
              className="font-mono text-sm"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label>{t("project.projectColor")}</Label>
              <div className="flex flex-wrap gap-1.5">
                {COLOR_CHOICES.map((c) => (
                  <button
                    key={c.value}
                    type="button"
                    onClick={() => setColor(c.value)}
                    className={`h-6 w-6 rounded-full ring-offset-background transition-all ${c.className} ${
                      color === c.value
                        ? "ring-2 ring-foreground ring-offset-2"
                        : "hover:scale-110"
                    }`}
                    aria-label={c.label}
                  />
                ))}
                <button
                  type="button"
                  onClick={() => setColor("")}
                  className={`h-6 w-6 rounded-full border border-dashed border-muted-foreground/50 text-xs text-muted-foreground ${
                    !color ? "ring-2 ring-foreground ring-offset-2" : ""
                  }`}
                  aria-label="no color"
                >
                  —
                </button>
              </div>
            </div>

            <div className="space-y-1.5">
              <Label>{t("project.defaultAgent")}</Label>
              <Select
                value={defaultAgentId || "__none__"}
                onValueChange={(v) => setDefaultAgentId(v === "__none__" ? "" : v)}
              >
                <SelectTrigger>
                  <SelectValue placeholder={t("project.inheritGlobal")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">{t("project.inheritGlobal")}</SelectItem>
                  {agents.map((a) => (
                    <SelectItem key={a.id} value={a.id}>
                      {a.emoji ? `${a.emoji} ` : ""}
                      {a.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={saving}
          >
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleSave}
            disabled={saving || !name.trim()}
            className={
              saveStatus === "saved"
                ? "bg-emerald-600 hover:bg-emerald-600"
                : saveStatus === "failed"
                  ? "bg-destructive hover:bg-destructive"
                  : ""
            }
          >
            {saving && <Loader2 className="mr-1 h-4 w-4 animate-spin" />}
            {saveStatus === "saved" && <Check className="mr-1 h-4 w-4" />}
            {saving
              ? t("common.saving")
              : saveStatus === "saved"
                ? t("common.saved")
                : t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
