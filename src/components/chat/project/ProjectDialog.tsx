/**
 * Create / edit project dialog.
 *
 * Reused for both flows:
 *  - `mode="create"` + `initialProject=undefined` → blank form, calls onCreate
 *  - `mode="edit"` + `initialProject=<Project>` → prefilled form, calls onUpdate
 */

import { useEffect, useRef, useState, type ChangeEvent } from "react"
import { useTranslation } from "react-i18next"
import {
  Bot,
  Camera,
  Check,
  CircleSlash,
  FileText,
  FolderKanban,
  FolderOpen,
  ImagePlus,
  Loader2,
  Palette,
  X,
} from "lucide-react"

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
import { cn } from "@/lib/utils"
import { formatBytes } from "@/lib/format"
import { isTauriMode } from "@/lib/transport"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"

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
  {
    value: "amber",
    label: "amber",
    className: "bg-amber-500",
    softClassName: "bg-amber-500/15",
  },
  {
    value: "violet",
    label: "violet",
    className: "bg-violet-500",
    softClassName: "bg-violet-500/15",
  },
  {
    value: "sky",
    label: "sky",
    className: "bg-sky-500",
    softClassName: "bg-sky-500/15",
  },
  {
    value: "emerald",
    label: "emerald",
    className: "bg-emerald-500",
    softClassName: "bg-emerald-500/15",
  },
  {
    value: "rose",
    label: "rose",
    className: "bg-rose-500",
    softClassName: "bg-rose-500/15",
  },
  {
    value: "indigo",
    label: "indigo",
    className: "bg-indigo-500",
    softClassName: "bg-indigo-500/15",
  },
  {
    value: "slate",
    label: "slate",
    className: "bg-slate-500",
    softClassName: "bg-slate-500/15",
  },
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
  const [workingDir, setWorkingDir] = useState<string>("")
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
      setWorkingDir(initialProject.workingDir ?? "")
    } else {
      setName("")
      setDescription("")
      setInstructions("")
      setEmoji("")
      setLogo("")
      setColor("")
      setDefaultAgentId("")
      setWorkingDir("")
    }
  }, [open, mode, initialProject])

  const selectedColor = COLOR_CHOICES.find((choice) => choice.value === color)

  async function handleLogoFileChange(e: ChangeEvent<HTMLInputElement>) {
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

  const {
    pick: pickWorkingDir,
    browserOpen: dirBrowserOpen,
    setBrowserOpen: setDirBrowserOpen,
    handleBrowserSelect: handleWorkingDirSelect,
  } = useDirectoryPicker({
    onPicked: setWorkingDir,
    errorTitle: t("project.workingDir.invalid"),
    loggerSource: "ProjectDialog::pickWorkingDir",
  })

  function handlePickWorkingDir() {
    if (saving) return
    void pickWorkingDir()
  }

  function clearWorkingDir() {
    setWorkingDir("")
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
          workingDir: workingDir.trim() || null,
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
          workingDir: workingDir.trim(),
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
      <DialogContent className="max-h-[88vh] max-w-3xl gap-0 overflow-hidden p-0">
        <div className="border-b border-border/70 bg-muted/25 px-6 py-5">
          <DialogHeader className="space-y-0">
            <DialogTitle className="flex items-center gap-3 text-xl">
              <span
                className={cn(
                  "flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-lg text-base shadow-sm",
                  selectedColor
                    ? `${selectedColor.softClassName} text-foreground`
                    : "bg-primary/10 text-primary",
                )}
              >
                {logo ? (
                  <img src={logo} alt="" className="h-full w-full object-cover" />
                ) : emoji ? (
                  <span>{emoji}</span>
                ) : (
                  <FolderKanban className="h-5 w-5" />
                )}
              </span>
              {mode === "create" ? t("project.newProject") : t("project.editProject")}
            </DialogTitle>
          </DialogHeader>
        </div>

        <div className="max-h-[calc(88vh-9.5rem)] overflow-y-auto px-6 py-5">
          <div className="grid gap-5 lg:grid-cols-[13rem_minmax(0,1fr)]">
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>{t("project.projectLogo")}</Label>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => fileInputRef.current?.click()}
                  className={cn(
                    "group relative h-40 w-full overflow-hidden rounded-lg border-dashed bg-muted/20 p-0 shadow-none hover:bg-muted/30",
                    selectedColor && !logo ? "border-border" : "",
                  )}
                  aria-label={t("project.uploadLogo")}
                >
                  {logo ? (
                    <img src={logo} alt="" className="h-full w-full object-cover" />
                  ) : (
                    <div className="flex h-full w-full flex-col items-center justify-center gap-3">
                      <div
                        className={cn(
                          "flex h-16 w-16 items-center justify-center rounded-lg text-3xl shadow-sm",
                          selectedColor
                            ? `${selectedColor.softClassName} text-foreground`
                            : "bg-background text-primary",
                        )}
                      >
                        {emoji ? (
                          <span>{emoji}</span>
                        ) : (
                          <ImagePlus className="h-7 w-7 text-muted-foreground" />
                        )}
                      </div>
                      <span className="text-xs font-medium text-muted-foreground">
                        {t("project.uploadLogo")}
                      </span>
                    </div>
                  )}
                  <span className="absolute inset-x-0 bottom-0 flex items-center justify-center gap-1 bg-background/90 px-3 py-2 text-xs font-medium text-foreground opacity-0 transition-opacity group-hover:opacity-100">
                    <Camera className="h-3.5 w-3.5" />
                    {logo ? t("project.replaceLogo") : t("project.uploadLogo")}
                  </span>
                </Button>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept="image/*"
                  className="hidden"
                  onChange={handleLogoFileChange}
                />
                <div className="flex min-h-8 items-start justify-between gap-2">
                  <p className="text-xs leading-5 text-muted-foreground">
                    {t("project.projectLogoHint")}
                  </p>
                  {logo && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={clearLogo}
                      className="h-7 shrink-0 px-2 text-muted-foreground"
                    >
                      <X className="mr-1 h-3.5 w-3.5" />
                      {t("project.removeLogo")}
                    </Button>
                  )}
                </div>
                {logoError && (
                  <p className="text-xs text-destructive">{logoError}</p>
                )}
              </div>

              <div className="space-y-2">
                <Label className="flex items-center gap-2">
                  <Palette className="h-4 w-4 text-muted-foreground" />
                  {t("project.projectColor")}
                </Label>
                <div className="grid grid-cols-4 gap-2">
                  {COLOR_CHOICES.map((choice) => (
                    <Button
                      key={choice.value}
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => setColor(choice.value)}
                      className={cn(
                        "h-9 w-9 rounded-full border border-transparent p-0 ring-offset-background transition-all hover:scale-105 hover:bg-transparent",
                        color === choice.value && "ring-2 ring-foreground ring-offset-2",
                      )}
                      aria-label={choice.label}
                    >
                      <span className={cn("h-6 w-6 rounded-full", choice.className)} />
                    </Button>
                  ))}
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() => setColor("")}
                    className={cn(
                      "h-9 w-9 rounded-full border border-dashed border-muted-foreground/40 p-0 text-muted-foreground hover:bg-muted/40",
                      !color && "ring-2 ring-foreground ring-offset-2",
                    )}
                    aria-label="no color"
                  >
                    <CircleSlash className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>

            <div className="space-y-5">
              <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_5rem]">
                <div className="space-y-1.5">
                  <Label htmlFor="project-name">{t("project.projectName")}</Label>
                  <Input
                    id="project-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t("project.projectNamePlaceholder")}
                    autoFocus
                    className="h-10"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="project-emoji">{t("project.projectEmoji")}</Label>
                  <Input
                    id="project-emoji"
                    value={emoji}
                    onChange={(e) => setEmoji(e.target.value)}
                    placeholder="🦭"
                    maxLength={4}
                    className="h-10 text-center text-lg"
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
                  rows={3}
                  className="resize-none"
                />
              </div>

              <div className="space-y-1.5">
                <Label className="flex items-center gap-2">
                  <Bot className="h-4 w-4 text-muted-foreground" />
                  {t("project.defaultAgent")}
                </Label>
                <Select
                  value={defaultAgentId || "__none__"}
                  onValueChange={(v) => setDefaultAgentId(v === "__none__" ? "" : v)}
                >
                  <SelectTrigger className="h-10">
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

              <div className="space-y-1.5">
                <Label className="flex items-center gap-2">
                  <FolderOpen className="h-4 w-4 text-muted-foreground" />
                  {t("project.workingDir.label")}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {t("project.workingDir.hint")}
                </p>
                <div className="flex items-center gap-2">
                  <Input
                    id="project-working-dir"
                    value={workingDir}
                    readOnly
                    placeholder={t("project.workingDir.placeholder")}
                    className="h-10 flex-1 font-mono text-xs"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    onClick={handlePickWorkingDir}
                    disabled={saving}
                  >
                    {t("project.workingDir.pick")}
                  </Button>
                  {workingDir && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={clearWorkingDir}
                      disabled={saving}
                      aria-label={t("project.workingDir.clear")}
                      className="h-9 w-9"
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </div>

              <div className="space-y-1.5">
                <Label htmlFor="project-instructions" className="flex items-center gap-2">
                  <FileText className="h-4 w-4 text-muted-foreground" />
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
                  rows={7}
                  className="max-h-52 min-h-36 font-mono text-sm"
                />
              </div>

              {error && (
                <p className="rounded-md border border-destructive/20 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                  {error}
                </p>
              )}
            </div>
          </div>
        </div>

        {!isTauriMode() && (
          <ServerDirectoryBrowser
            open={dirBrowserOpen}
            initialPath={workingDir || null}
            onOpenChange={setDirBrowserOpen}
            onSelect={handleWorkingDirSelect}
          />
        )}

        <DialogFooter className="border-t border-border/70 bg-background px-6 py-4">
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
      `Image too large (max ${formatBytes(MAX_LOGO_SOURCE_BYTES, {
        unit: "MB",
        fractionDigits: 0,
      })})`,
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
