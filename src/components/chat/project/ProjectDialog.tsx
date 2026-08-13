/**
 * Create / edit project dialog.
 *
 * Reused for both flows:
 *  - `mode="create"` + `initialProject=undefined` → blank form, calls onCreate
 *  - `mode="edit"` + `initialProject=<Project>` → prefilled form, calls onUpdate
 */

import { useCallback, useEffect, useRef, useState, type ChangeEvent } from "react"
import { useTranslation } from "react-i18next"
import {
  Bot,
  Camera,
  Check,
  CircleSlash,
  FolderOpen,
  FolderPlus,
  ImagePlus,
  Loader2,
  MoreHorizontal,
  Palette,
  Star,
  Trash2,
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
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  AgentSelectDisplay,
  INHERIT_AGENT_SENTINEL,
  InheritAgentSelectDisplay,
} from "@/components/common/AgentSelectDisplay"
import { cn } from "@/lib/utils"
import { formatBytes } from "@/lib/format"
import { getTransport, useTransport } from "@/lib/transport-provider"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"
import ProjectInstructionsField from "./ProjectInstructionsField"
import ProjectKnowledgeSection from "./ProjectKnowledgeSection"
import { shouldSubmitProjectInstructions } from "./projectInstructionsDraft"
import { promoteProjectSourceFolder } from "./projectSourceFolders"
import { useProjectWorkingDir } from "./hooks/useProjectWorkingDir"

import type {
  CreateProjectInput,
  Project,
  ProjectInstructionsDraft,
  ProjectInstructionsFile,
  UpdateProjectInput,
} from "@/types/project"
import type { AgentSummaryForSidebar } from "@/types/chat"

export interface ProjectDialogProps {
  open: boolean
  mode: "create" | "edit"
  initialProject?: Project | null
  agents: AgentSummaryForSidebar[]
  onOpenChange: (open: boolean) => void
  onCreate?: (
    input: CreateProjectInput,
    instructions: ProjectInstructionsDraft | undefined,
    createInstructionsIfMissing: boolean,
  ) => Promise<Project | null>
  onUpdate?: (
    id: string,
    patch: UpdateProjectInput,
    instructions?: ProjectInstructionsDraft,
  ) => Promise<Project | null>
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
  const transport = useTransport()

  const [name, setName] = useState("")
  const [description, setDescription] = useState("")
  const [logo, setLogo] = useState<string>("")
  const [color, setColor] = useState<string>("")
  const [defaultAgentId, setDefaultAgentId] = useState<string>("")
  const [workingDir, setWorkingDir] = useState<string>("")
  const [linkedDirs, setLinkedDirs] = useState<string[]>([])
  const [instructions, setInstructions] = useState("")
  const [loadedInstructions, setLoadedInstructions] = useState("")
  const [instructionsHash, setInstructionsHash] = useState("")
  const [instructionsPath, setInstructionsPath] = useState("AGENTS.md")
  const [instructionsLoading, setInstructionsLoading] = useState(false)
  const [instructionsReady, setInstructionsReady] = useState(false)
  const [instructionsExists, setInstructionsExists] = useState(false)
  const [createInstructionsIfMissing, setCreateInstructionsIfMissing] = useState(true)
  const [instructionsError, setInstructionsError] = useState("")
  const instructionsRequestSeq = useRef(0)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [logoError, setLogoError] = useState("")

  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [error, setError] = useState("")
  const selectedDefaultAgent = agents.find((agent) => agent.id === defaultAgentId)
  const effectivePrimaryDir = useProjectWorkingDir(
    transport,
    mode === "edit" ? (initialProject?.id ?? null) : null,
    workingDir || null,
  )

  const resetInstructionsForNewWorkspace = useCallback(() => {
    instructionsRequestSeq.current += 1
    setInstructions("")
    setLoadedInstructions("")
    setInstructionsHash("")
    setInstructionsPath("AGENTS.md")
    setInstructionsLoading(false)
    setInstructionsReady(true)
    setInstructionsExists(false)
    setCreateInstructionsIfMissing(true)
    setInstructionsError("")
  }, [])

  const loadInstructions = useCallback(
    async (
      command: "get_project_instructions_cmd" | "inspect_project_instructions_cmd",
      args: Record<string, unknown>,
      pendingPath: string,
    ) => {
      const seq = ++instructionsRequestSeq.current
      setInstructionsLoading(true)
      setInstructionsReady(false)
      setInstructions("")
      setLoadedInstructions("")
      setInstructionsHash("")
      setInstructionsExists(false)
      setCreateInstructionsIfMissing(true)
      setInstructionsError("")
      setInstructionsPath(pendingPath)
      try {
        const file = await getTransport().call<ProjectInstructionsFile>(command, args)
        if (seq !== instructionsRequestSeq.current) return
        setInstructions(file.content)
        setLoadedInstructions(file.content)
        setInstructionsHash(file.contentHash)
        setInstructionsPath(file.path)
        setInstructionsExists(file.exists)
        setInstructionsReady(true)
      } catch (loadError) {
        if (seq !== instructionsRequestSeq.current) return
        setInstructionsError(loadError instanceof Error ? loadError.message : String(loadError))
      } finally {
        if (seq === instructionsRequestSeq.current) setInstructionsLoading(false)
      }
    },
    [],
  )

  useEffect(() => {
    if (!open) return
    setError("")
    setLogoError("")
    setSaveStatus("idle")
    if (mode === "edit" && initialProject) {
      setName(initialProject.name ?? "")
      setDescription(initialProject.description ?? "")
      setLogo(initialProject.logo ?? "")
      setColor(initialProject.color ?? "")
      setDefaultAgentId(initialProject.defaultAgentId ?? "")
      setWorkingDir(initialProject.workingDir ?? "")
      setLinkedDirs(initialProject.linkedDirs ?? [])
    } else {
      setName("")
      setDescription("")
      setLogo("")
      setColor("")
      setDefaultAgentId("")
      setWorkingDir("")
      setLinkedDirs([])
    }
  }, [open, mode, initialProject])

  useEffect(() => {
    if (!open) {
      instructionsRequestSeq.current += 1
      return
    }
    if (mode === "edit" && initialProject?.id) {
      void loadInstructions("get_project_instructions_cmd", { id: initialProject.id }, "AGENTS.md")
    } else {
      resetInstructionsForNewWorkspace()
    }
    return () => {
      instructionsRequestSeq.current += 1
    }
  }, [initialProject?.id, loadInstructions, mode, open, resetInstructionsForNewWorkspace])

  const selectedColor = COLOR_CHOICES.find((choice) => choice.value === color)
  const canSkipMissingInstructions =
    mode === "create" && Boolean(workingDir.trim()) && instructionsReady && !instructionsExists

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

  const makePrimaryDir = useCallback(
    (path: string) => {
      const previousPrimary = effectivePrimaryDir?.trim() ?? ""
      if (previousPrimary === path) return
      const promoted = promoteProjectSourceFolder(path, previousPrimary, linkedDirs)
      setWorkingDir(promoted.workingDir)
      setLinkedDirs(promoted.linkedDirs)
      void loadInstructions(
        "inspect_project_instructions_cmd",
        { workingDir: path, projectId: null },
        agentsFilePath(path),
      )
      if (mode !== "create") return

      const inferredName = getDirectoryName(path)
      if (!inferredName) return

      setName((currentName) => (currentName.trim() ? currentName : inferredName))
    },
    [effectivePrimaryDir, linkedDirs, loadInstructions, mode],
  )

  const handleSourceDirPicked = useCallback(
    (path: string) => {
      const trimmedPath = path.trim()
      if (!trimmedPath || trimmedPath === workingDir || linkedDirs.includes(trimmedPath)) return

      // A newly-created project follows Codex's first-folder behavior: the
      // first attached folder becomes primary. Existing projects with an
      // implicit default workspace keep it primary until the user explicitly
      // promotes a newly attached folder.
      if (mode === "create" && !workingDir.trim() && linkedDirs.length === 0) {
        makePrimaryDir(trimmedPath)
        return
      }
      setLinkedDirs((current) => [...current, trimmedPath])
    },
    [linkedDirs, makePrimaryDir, mode, workingDir],
  )

  const {
    pick: pickSourceDir,
    browserOpen: sourceDirBrowserOpen,
    setBrowserOpen: setSourceDirBrowserOpen,
    handleBrowserSelect: handleSourceDirSelect,
  } = useDirectoryPicker({
    onPicked: handleSourceDirPicked,
    errorTitle: t("project.linkedDirs.invalid"),
    loggerSource: "ProjectDialog::pickSourceDir",
  })

  function handleAddSourceDir() {
    if (saving) return
    void pickSourceDir()
  }

  function handleCreateSourceDir() {
    if (saving) return
    setSourceDirBrowserOpen(true)
  }

  function clearWorkingDir() {
    setWorkingDir("")
    if (mode === "edit" && initialProject?.id) {
      void loadInstructions(
        "inspect_project_instructions_cmd",
        { workingDir: null, projectId: initialProject.id },
        "AGENTS.md",
      )
    } else {
      resetInstructionsForNewWorkspace()
    }
  }

  function removeLinkedDir(path: string) {
    setLinkedDirs((current) => current.filter((linked) => linked !== path))
  }

  function removePrimaryDir() {
    const [nextPrimary, ...remainingLinked] = linkedDirs
    if (nextPrimary) {
      setWorkingDir(nextPrimary)
      setLinkedDirs(remainingLinked)
      void loadInstructions(
        "inspect_project_instructions_cmd",
        { workingDir: nextPrimary, projectId: null },
        agentsFilePath(nextPrimary),
      )
      return
    }
    clearWorkingDir()
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
        const shouldCreateInstructions = !canSkipMissingInstructions || createInstructionsIfMissing
        const created = await onCreate(
          {
            name: name.trim(),
            description: description.trim() || null,
            logo: logo || null,
            color: color || null,
            defaultAgentId: defaultAgentId || null,
            workingDir: workingDir.trim() || null,
            linkedDirs,
          },
          shouldCreateInstructions
            ? {
                content: instructions,
                expectedFileHash: instructionsHash,
                expectedExists: instructionsExists,
              }
            : undefined,
          shouldCreateInstructions,
        )
        if (created) {
          setSaveStatus("saved")
          setTimeout(() => onOpenChange(false), 400)
        } else {
          setSaveStatus("failed")
        }
      } else if (mode === "edit" && initialProject && onUpdate) {
        const instructionsDraft = shouldSubmitProjectInstructions(
          instructions,
          loadedInstructions,
          workingDir,
          initialProject.workingDir,
        )
          ? {
              content: instructions,
              expectedFileHash: instructionsHash,
              expectedExists: instructionsExists,
            }
          : undefined
        const updated = await onUpdate(
          initialProject.id,
          {
            name: name.trim(),
            description: description.trim(),
            logo: logo,
            color: color,
            defaultAgentId: defaultAgentId,
            workingDir: workingDir.trim(),
            linkedDirs,
          },
          instructionsDraft,
        )
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
              {logo && (
                <span
                  className={cn(
                    "flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-lg text-base shadow-sm",
                    selectedColor
                      ? `${selectedColor.softClassName} text-foreground`
                      : "bg-primary/10 text-primary",
                  )}
                >
                  <img src={logo} alt="" className="h-full w-full object-cover" />
                </span>
              )}
              {mode === "create" ? t("project.newProject") : t("project.editProject")}
            </DialogTitle>
          </DialogHeader>
        </div>

        <div className="max-h-[calc(88vh-9.5rem)] overflow-y-auto px-6 py-5">
          <div className="mb-5 border-b border-border/70 pb-5">
            <div className="mb-3 flex items-start gap-3">
              <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/70 bg-muted/30 text-muted-foreground">
                <FolderOpen className="h-4 w-4" />
              </span>
              <div className="min-w-0 flex-1">
                <Label className="text-sm font-semibold">{t("project.linkedDirs.label")}</Label>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {t("project.linkedDirs.hint")}
                </p>
              </div>
            </div>
            <div className="mb-2 flex flex-wrap justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleAddSourceDir}
                disabled={saving}
                className="shadow-none"
              >
                <FolderPlus className="mr-1.5 h-4 w-4" />
                {t("project.linkedDirs.add")}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleCreateSourceDir}
                disabled={saving}
                className="shadow-none"
              >
                <FolderPlus className="mr-1.5 h-4 w-4" />
                {t("fileBrowser.newFolder", { defaultValue: "New folder" })}
              </Button>
            </div>
            <div className="overflow-hidden rounded-lg border border-border/70 bg-muted/10">
              <div className="flex min-h-11 items-center gap-2 border-b border-border/60 px-3">
                <FolderOpen className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span
                  className={cn(
                    "min-w-0 flex-1 truncate text-xs",
                    workingDir ? "font-mono" : "text-muted-foreground",
                  )}
                >
                  {workingDir || t("project.linkedDirs.defaultPrimary")}
                </span>
                <span className="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                  {t("project.linkedDirs.primary")}
                </span>
                {workingDir ? (
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={saving}
                        aria-label={t("project.linkedDirs.folderActions")}
                        className="h-8 w-8 shrink-0"
                      >
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onSelect={removePrimaryDir}
                        className="text-destructive focus:text-destructive"
                      >
                        <Trash2 className="mr-2 h-4 w-4" />
                        {t("project.linkedDirs.removeFolder")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : null}
              </div>
              {linkedDirs.map((path) => (
                <div
                  key={path}
                  className="flex min-h-11 items-center gap-2 border-b border-border/60 px-3 last:border-b-0"
                >
                  <FolderOpen className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">{path}</span>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={saving}
                        aria-label={t("project.linkedDirs.folderActions")}
                        className="h-8 w-8 shrink-0"
                      >
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        disabled={mode === "edit" && !effectivePrimaryDir}
                        onSelect={() => makePrimaryDir(path)}
                      >
                        <Star className="mr-2 h-4 w-4" />
                        {t("project.linkedDirs.makePrimary")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onSelect={() => removeLinkedDir(path)}
                        className="text-destructive focus:text-destructive"
                      >
                        <Trash2 className="mr-2 h-4 w-4" />
                        {t("project.linkedDirs.removeFolder")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              ))}
            </div>
          </div>

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
                        <ImagePlus className="h-7 w-7 text-muted-foreground" />
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
                {logoError && <p className="text-xs text-destructive">{logoError}</p>}
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
                        "h-9 w-9 rounded-full border border-transparent p-0 transition-colors hover:bg-secondary/40",
                        color === choice.value && "bg-secondary",
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
                      !color && "bg-secondary",
                    )}
                    aria-label={t("common.none")}
                  >
                    <CircleSlash className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>

            <div className="space-y-5">
              <div className="space-y-1.5">
                <Label htmlFor="project-name">{t("project.projectName")}</Label>
                <Input
                  id="project-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t("project.projectNamePlaceholder")}
                  className="h-10"
                />
              </div>

              <div className="space-y-1.5">
                <Label htmlFor="project-description">{t("project.projectDescription")}</Label>
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
                  value={defaultAgentId || INHERIT_AGENT_SENTINEL}
                  onValueChange={(v) => setDefaultAgentId(v === INHERIT_AGENT_SENTINEL ? "" : v)}
                >
                  <SelectTrigger className="h-10">
                    {selectedDefaultAgent ? (
                      <AgentSelectDisplay agent={selectedDefaultAgent} />
                    ) : (
                      <InheritAgentSelectDisplay label={t("project.inheritGlobal")} />
                    )}
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem
                      value={INHERIT_AGENT_SENTINEL}
                      textValue={t("project.inheritGlobal")}
                    >
                      {t("project.inheritGlobal")}
                    </SelectItem>
                    {agents.map((a) => (
                      <SelectItem key={a.id} value={a.id} textValue={a.name}>
                        <AgentSelectDisplay agent={a} />
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {mode === "edit" && initialProject?.id && (
                <ProjectKnowledgeSection projectId={initialProject.id} />
              )}

              {canSkipMissingInstructions && (
                <div className="flex items-start justify-between gap-4 rounded-lg border border-border/70 bg-muted/20 px-4 py-3">
                  <div className="min-w-0">
                    <Label htmlFor="project-create-agents-md" className="text-sm font-medium">
                      {t("project.createAgentsFile")}
                    </Label>
                    <p className="mt-1 text-xs leading-5 text-muted-foreground">
                      {t("project.createAgentsFileHint")}
                    </p>
                  </div>
                  <Switch
                    id="project-create-agents-md"
                    checked={createInstructionsIfMissing}
                    onCheckedChange={setCreateInstructionsIfMissing}
                    disabled={saving}
                    aria-label={t("project.createAgentsFile")}
                    className="mt-0.5"
                  />
                </div>
              )}

              {(!canSkipMissingInstructions || createInstructionsIfMissing) && (
                <ProjectInstructionsField
                  key={instructionsPath}
                  value={instructions}
                  path={instructionsPath}
                  loading={instructionsLoading}
                  error={instructionsError}
                  disabled={saving || !instructionsReady}
                  onChange={setInstructions}
                />
              )}

              {error && (
                <p className="rounded-md border border-destructive/20 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                  {error}
                </p>
              )}
            </div>
          </div>
        </div>

        <ServerDirectoryBrowser
          open={sourceDirBrowserOpen}
          initialPath={workingDir || null}
          onOpenChange={setSourceDirBrowserOpen}
          onSelect={handleSourceDirSelect}
          allowCreate
        />

        <DialogFooter className="border-t border-border/70 bg-background px-6 py-4">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleSave}
            disabled={saving || instructionsLoading || !instructionsReady || !name.trim()}
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

function getDirectoryName(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, "")
  if (!trimmed) return ""
  const segments = trimmed.split(/[\\/]+/)
  return segments[segments.length - 1] ?? ""
}

function agentsFilePath(root: string): string {
  const trimmed = root.trim().replace(/[\\/]+$/, "")
  if (!trimmed) return "AGENTS.md"
  const separator = trimmed.includes("\\") && !trimmed.includes("/") ? "\\" : "/"
  return `${trimmed}${separator}AGENTS.md`
}

async function resizeImageToDataUrl(file: File, maxSize: number, quality: number): Promise<string> {
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
