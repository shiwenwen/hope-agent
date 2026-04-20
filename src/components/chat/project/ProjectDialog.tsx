/**
 * Create / edit project dialog.
 *
 * Reused for both flows:
 *  - `mode="create"` + `initialProject=undefined` → blank form, calls onCreate
 *  - `mode="edit"` + `initialProject=<Project>` → prefilled form, calls onUpdate
 */

import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Check, Camera } from "lucide-react"

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
  const [logo, setLogo] = useState<string>("")
  const [color, setColor] = useState<string>("")
  const [defaultAgentId, setDefaultAgentId] = useState<string>("")
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [logoError, setLogoError] = useState("")

  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">(
    "idle",
  )
  const [error, setError] = useState("")

  useEffect(() => {
    if (!open) return
    setError("")
    setLogoError("")
    setSaveStatus("idle")
    if (mode === "edit" && initialProject) {
      setName(initialProject.name ?? "")
      setDescription(initialProject.description ?? "")
      setInstructions(initialProject.instructions ?? "")
      setEmoji(initialProject.emoji ?? "")
      setLogo(initialProject.logo ?? "")
      setColor(initialProject.color ?? "")
      setDefaultAgentId(initialProject.defaultAgentId ?? "")
    } else {
      setName("")
      setDescription("")
      setInstructions("")
      setEmoji("")
      setLogo("")
      setColor("")
      setDefaultAgentId("")
    }
  }, [open, mode, initialProject])

  async function handleLogoFileChange(
    e: React.ChangeEvent<HTMLInputElement>,
  ) {
    const file = e.target.files?.[0]
    // Reset the input so re-selecting the same file still fires change.
    e.target.value = ""
    if (!file) return
    setLogoError("")
    try {
      const dataUrl = await resizeImageToDataUrl(file, 256, 0.85)
      setLogo(dataUrl)
    } catch (err) {
      setLogoError(err instanceof Error ? err.message : String(err))
    }
  }

  function clearLogo() {
    setLogo("")
    setLogoError("")
  }

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
          logo: logo || null,
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
          logo: logo,
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
          </div>

          <div className="space-y-1.5">
            <Label>{t("project.projectLogo")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("project.projectLogoHint")}
            </p>
            <div className="flex items-center gap-3">
              <div
                onClick={() => fileInputRef.current?.click()}
                className="w-14 h-14 rounded-xl bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors cursor-pointer shrink-0"
                role="button"
                aria-label={t("project.uploadLogo")}
              >
                {logo ? (
                  <img src={logo} alt="" className="w-full h-full object-cover" />
                ) : emoji ? (
                  <span className="text-2xl">{emoji}</span>
                ) : (
                  <Camera className="h-5 w-5 text-muted-foreground/40" />
                )}
              </div>
              {logo && (
                <button
                  type="button"
                  onClick={clearLogo}
                  className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                >
                  {t("project.removeLogo")}
                </button>
              )}
              <input
                ref={fileInputRef}
                type="file"
                accept="image/*"
                className="hidden"
                onChange={handleLogoFileChange}
              />
            </div>
            {logoError && (
              <p className="text-xs text-destructive">{logoError}</p>
            )}
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

/** Hard cap on raw upload size before decoding — guards against oversized
 * images crashing the canvas decoder. 8 MB is comfortable for any real logo. */
const MAX_LOGO_SOURCE_BYTES = 8 * 1024 * 1024

async function resizeImageToDataUrl(
  file: File,
  maxSize: number,
  quality: number,
): Promise<string> {
  if (file.size > MAX_LOGO_SOURCE_BYTES) {
    throw new Error(
      `Image too large (max ${(MAX_LOGO_SOURCE_BYTES / 1024 / 1024).toFixed(0)}MB)`,
    )
  }
  const img = await loadImageFromFile(file)
  const srcW = img.naturalWidth || img.width
  const srcH = img.naturalHeight || img.height
  const ratio = Math.min(1, maxSize / Math.max(srcW, srcH))
  const w = Math.max(1, Math.round(srcW * ratio))
  const h = Math.max(1, Math.round(srcH * ratio))
  const canvas = document.createElement("canvas")
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext("2d")
  if (!ctx) throw new Error("Canvas context unavailable")
  ctx.drawImage(img, 0, 0, w, h)
  // WebP is ~30% smaller than JPEG at equivalent quality and supported by
  // Tauri's WebView / modern browsers. Fall back to JPEG if encoding fails.
  let dataUrl = canvas.toDataURL("image/webp", quality)
  if (!dataUrl.startsWith("data:image/webp")) {
    dataUrl = canvas.toDataURL("image/jpeg", quality)
  }
  return dataUrl
}

function loadImageFromFile(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file)
    const img = new Image()
    img.onload = () => {
      URL.revokeObjectURL(url)
      resolve(img)
    }
    img.onerror = () => {
      URL.revokeObjectURL(url)
      reject(new Error("Failed to decode image"))
    }
    img.src = url
  })
}
