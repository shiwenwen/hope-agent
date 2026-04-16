import { useState, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  Download,
  FileText,
  Loader2,
  X,
} from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface OpenClawAgentPreview {
  id: string
  name: string
  emoji: string | null
  theme: string | null
  avatar: string | null
  modelInfo: string | null
  hasSystemPrompt: boolean
  sandbox: boolean
  skillNames: string[]
  availableFiles: string[]
  alreadyExists: boolean
}

interface ImportAgentRequest {
  sourceId: string
  targetId: string
  name: string
  emoji: string | null
  vibe: string | null
  sandbox: boolean
  importFiles: string[]
}

interface ImportResult {
  sourceId: string
  importedId: string
  name: string
  success: boolean
  error: string | null
}

interface EditableAgent {
  sourceId: string
  targetId: string
  name: string
  emoji: string
  vibe: string
  sandbox: boolean
  importFiles: string[]
  availableFiles: string[]
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImported: () => void
}

function TickBox({ checked, size = "md" }: { checked: boolean; size?: "sm" | "md" }) {
  const dims = size === "sm" ? "w-4 h-4" : "w-5 h-5"
  const iconSize = size === "sm" ? "h-2.5 w-2.5" : "h-3 w-3"
  const border = size === "sm" ? "border" : "border-2"
  return (
    <div
      className={`${dims} rounded ${border} flex items-center justify-center shrink-0 transition-colors ${
        checked ? "border-primary bg-primary" : "border-muted-foreground/30"
      }`}
    >
      {checked && <Check className={`${iconSize} text-primary-foreground`} />}
    </div>
  )
}

type Step = "scan" | "edit" | "result"

export default function OpenClawImportDialog({ open, onOpenChange, onImported }: Props) {
  const { t } = useTranslation()

  const [step, setStep] = useState<Step>("scan")
  const [scanning, setScanning] = useState(false)
  const [previews, setPreviews] = useState<OpenClawAgentPreview[]>([])
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [editables, setEditables] = useState<EditableAgent[]>([])
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const [importing, setImporting] = useState(false)
  const [results, setResults] = useState<ImportResult[]>([])
  const [error, setError] = useState<string | null>(null)

  const doScan = useCallback(async () => {
    setScanning(true)
    setError(null)
    try {
      const list = await getTransport().call<OpenClawAgentPreview[]>("scan_openclaw_agents")
      setPreviews(list)
      setSelected(new Set(list.map((a) => a.id)))
    } catch (e) {
      setError(String(e))
      logger.error("settings", "OpenClawImport::scan", "Failed to scan", e)
    } finally {
      setScanning(false)
    }
  }, [])

  const handleOpen = useCallback(
    (isOpen: boolean) => {
      onOpenChange(isOpen)
      if (isOpen) {
        setStep("scan")
        setPreviews([])
        setSelected(new Set())
        setEditables([])
        setExpandedIds(new Set())
        setResults([])
        setError(null)
        doScan()
      }
    },
    [onOpenChange, doScan],
  )

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const goToEdit = () => {
    const selectedPreviews = previews.filter((p) => selected.has(p.id))
    setEditables(
      selectedPreviews.map((p) => ({
        sourceId: p.id,
        targetId: p.alreadyExists ? `${p.id}-oc` : p.id,
        name: p.name,
        emoji: p.emoji ?? "",
        vibe: p.theme ?? "",
        sandbox: p.sandbox,
        importFiles: [...p.availableFiles],
        availableFiles: [...p.availableFiles],
      })),
    )
    setExpandedIds(new Set())
    setStep("edit")
  }

  const updateEditable = (idx: number, patch: Partial<EditableAgent>) => {
    setEditables((prev) => prev.map((e, i) => (i === idx ? { ...e, ...patch } : e)))
  }

  const toggleFile = (idx: number, file: string) => {
    setEditables((prev) =>
      prev.map((e, i) => {
        if (i !== idx) return e
        const files = e.importFiles.includes(file)
          ? e.importFiles.filter((f) => f !== file)
          : [...e.importFiles, file]
        return { ...e, importFiles: files }
      }),
    )
  }

  const toggleExpanded = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const doImport = async () => {
    setImporting(true)
    setError(null)
    try {
      const requests: ImportAgentRequest[] = editables.map((e) => ({
        sourceId: e.sourceId,
        targetId: e.targetId,
        name: e.name,
        emoji: e.emoji || null,
        vibe: e.vibe || null,
        sandbox: e.sandbox,
        importFiles: e.importFiles,
      }))
      const res = await getTransport().call<ImportResult[]>("import_openclaw_agents", { requests })
      setResults(res)
      setStep("result")
      if (res.some((r) => r.success)) {
        window.dispatchEvent(new Event("agents-changed"))
        onImported()
      }
    } catch (e) {
      setError(String(e))
      logger.error("settings", "OpenClawImport::import", "Failed to import", e)
    } finally {
      setImporting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpen}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {step === "edit" && (
              <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => setStep("scan")}>
                <ArrowLeft className="h-4 w-4" />
              </Button>
            )}
            <Download className="h-5 w-5" />
            {t("settings.openclawImportTitle")}
          </DialogTitle>
          <DialogDescription>{t("settings.openclawImportDesc")}</DialogDescription>
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-y-auto py-2">
          {error && (
            <div className="rounded-lg bg-destructive/10 text-destructive text-sm px-3 py-2 mb-3">
              {error}
            </div>
          )}

          {step === "scan" && <ScanStep scanning={scanning} previews={previews} selected={selected} toggleSelect={toggleSelect} />}
          {step === "edit" && (
            <EditStep
              editables={editables}
              expandedIds={expandedIds}
              updateEditable={updateEditable}
              toggleFile={toggleFile}
              toggleExpanded={toggleExpanded}
            />
          )}
          {step === "result" && <ResultStep results={results} />}
        </div>

        <DialogFooter>
          {step === "scan" && (
            <Button onClick={goToEdit} disabled={scanning || selected.size === 0}>
              {t("settings.openclawImportNext")}
              <ChevronRight className="h-4 w-4 ml-1" />
            </Button>
          )}
          {step === "edit" && (
            <Button onClick={doImport} disabled={importing || editables.length === 0}>
              {importing ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : <Download className="h-4 w-4 mr-1" />}
              {t("settings.openclawImportBtn")}
            </Button>
          )}
          {step === "result" && (
            <Button onClick={() => handleOpen(false)}>{t("common.done")}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function ScanStep({
  scanning,
  previews,
  selected,
  toggleSelect,
}: {
  scanning: boolean
  previews: OpenClawAgentPreview[]
  selected: Set<string>
  toggleSelect: (id: string) => void
}) {
  const { t } = useTranslation()

  if (scanning) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        <span className="ml-2 text-sm text-muted-foreground">{t("settings.openclawImportScanning")}</span>
      </div>
    )
  }

  if (previews.length === 0) {
    return (
      <div className="text-center py-12">
        <p className="text-sm text-muted-foreground">{t("settings.openclawImportNoAgents")}</p>
        <p className="text-xs text-muted-foreground/60 mt-1">{t("settings.openclawImportNoAgentsHint")}</p>
      </div>
    )
  }

  return (
    <div className="space-y-1">
      {previews.map((agent) => (
        <button
          key={agent.id}
          className={`flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors ${
            selected.has(agent.id) ? "bg-primary/10 text-foreground" : "text-muted-foreground hover:bg-secondary/60"
          }`}
          onClick={() => toggleSelect(agent.id)}
        >
          <TickBox checked={selected.has(agent.id)} />

          <div className="w-8 h-8 rounded-full bg-primary/15 flex items-center justify-center text-base shrink-0">
            {agent.emoji ?? "🤖"}
          </div>

          <div className="flex-1 text-left min-w-0">
            <div className="font-medium truncate">{agent.name}</div>
            <div className="text-xs text-muted-foreground truncate">
              {agent.modelInfo && <span className="mr-2">{agent.modelInfo}</span>}
              {agent.skillNames.length > 0 && (
                <span>
                  {agent.skillNames.length} {t("settings.openclawImportSkills")}
                </span>
              )}
            </div>
          </div>

          {agent.alreadyExists && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-600 shrink-0">
              {t("settings.openclawImportExists")}
            </span>
          )}
        </button>
      ))}
    </div>
  )
}

function EditStep({
  editables,
  expandedIds,
  updateEditable,
  toggleFile,
  toggleExpanded,
}: {
  editables: EditableAgent[]
  expandedIds: Set<string>
  updateEditable: (idx: number, patch: Partial<EditableAgent>) => void
  toggleFile: (idx: number, file: string) => void
  toggleExpanded: (id: string) => void
}) {
  const { t } = useTranslation()

  return (
    <div className="space-y-4">
      {editables.map((agent, idx) => {
        const isExpanded = expandedIds.has(agent.sourceId)
        return (
          <div key={agent.sourceId} className="rounded-lg border border-border p-4 space-y-3">
            <div className="flex items-center gap-2">
              <span className="text-lg">{agent.emoji || "🤖"}</span>
              <span className="font-medium text-sm">{agent.name}</span>
              <span className="text-xs text-muted-foreground">({agent.sourceId})</span>
            </div>

            <div>
              <label className="text-xs font-medium text-muted-foreground mb-1 block">
                {t("settings.openclawImportFieldId")}
              </label>
              <Input
                className="bg-secondary/40 rounded-lg font-mono text-sm"
                value={agent.targetId}
                onChange={(e) => updateEditable(idx, { targetId: e.target.value })}
              />
            </div>

            <div>
              <label className="text-xs font-medium text-muted-foreground mb-1 block">
                {t("settings.agentName")}
              </label>
              <Input
                className="bg-secondary/40 rounded-lg text-sm"
                value={agent.name}
                onChange={(e) => updateEditable(idx, { name: e.target.value })}
              />
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">Emoji</label>
                <Input
                  className="bg-secondary/40 rounded-lg text-sm"
                  value={agent.emoji}
                  onChange={(e) => updateEditable(idx, { emoji: e.target.value })}
                />
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">
                  {t("settings.openclawImportFieldVibe")}
                </label>
                <Input
                  className="bg-secondary/40 rounded-lg text-sm"
                  value={agent.vibe}
                  onChange={(e) => updateEditable(idx, { vibe: e.target.value })}
                />
              </div>
            </div>

            <div className="flex items-center justify-between">
              <label className="text-xs font-medium text-muted-foreground">Sandbox</label>
              <Switch
                checked={agent.sandbox}
                onCheckedChange={(v) => updateEditable(idx, { sandbox: v })}
              />
            </div>

            {agent.availableFiles.length > 0 && (
              <div>
                <button
                  className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors"
                  onClick={() => toggleExpanded(agent.sourceId)}
                >
                  {isExpanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                  {t("settings.openclawImportFiles")} ({agent.importFiles.length}/{agent.availableFiles.length})
                </button>
                {isExpanded && (
                  <div className="mt-2 space-y-1 ml-4">
                    {agent.availableFiles.map((file) => {
                      const isSelected = agent.importFiles.includes(file)
                      return (
                        <button
                          key={file}
                          className={`flex items-center gap-2 w-full px-2 py-1 rounded text-xs transition-colors ${
                            isSelected ? "text-foreground bg-primary/10" : "text-muted-foreground hover:bg-secondary/60"
                          }`}
                          onClick={() => toggleFile(idx, file)}
                        >
                          <TickBox checked={isSelected} size="sm" />
                          <FileText className="h-3 w-3 shrink-0" />
                          <span>{file}</span>
                        </button>
                      )
                    })}
                  </div>
                )}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

function ResultStep({ results }: { results: ImportResult[] }) {
  const { t } = useTranslation()
  const successCount = results.filter((r) => r.success).length
  const failCount = results.length - successCount

  return (
    <div className="space-y-3">
      <div className="text-sm font-medium">
        {failCount === 0
          ? t("settings.openclawImportSuccess", { count: successCount })
          : t("settings.openclawImportPartial", { success: successCount, total: results.length })}
      </div>
      <div className="space-y-1">
        {results.map((r) => (
          <div
            key={r.importedId}
            className={`flex items-center gap-2 px-3 py-2 rounded-lg text-sm ${
              r.success ? "bg-green-500/10 text-green-600" : "bg-destructive/10 text-destructive"
            }`}
          >
            {r.success ? <Check className="h-4 w-4 shrink-0" /> : <X className="h-4 w-4 shrink-0" />}
            <span className="font-medium">{r.name}</span>
            <span className="text-xs text-muted-foreground">→ {r.importedId}</span>
            {r.error && <span className="text-xs ml-auto">{r.error}</span>}
          </div>
        ))}
      </div>
    </div>
  )
}
