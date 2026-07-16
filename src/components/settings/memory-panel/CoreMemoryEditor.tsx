import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { AlertCircle, Loader2, Check, Save, ChevronDown, ChevronRight } from "lucide-react"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import {
  coreMemoryOperationErrorToast,
  coreMemoryOperationForScope,
  type CoreMemoryOperationErrorToast,
} from "./coreMemoryOperationFeedback"

interface CoreMemoryEditorProps {
  scope: "global" | "agent" | "project"
  agentId?: string
  projectId?: string
}

interface CoreMemoryIndex {
  content?: string | null
  fileHash?: string | null
  state: string
  canonicalPath: string
  legacyPath?: string | null
}

interface CoreMemoryConflict {
  canonicalContent: string
  canonicalHash: string
  legacyContent: string
  legacyHash: string
  lastSyncedContent?: string | null
  canonicalPath: string
  legacyPath: string
}

export default function CoreMemoryEditor({ scope, agentId, projectId }: CoreMemoryEditorProps) {
  const { t } = useTranslation()
  const [content, setContent] = useState("")
  const [originalContent, setOriginalContent] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<CoreMemoryOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [expanded, setExpanded] = useState(true)
  const [fileHash, setFileHash] = useState<string | null>(null)
  const [migrationState, setMigrationState] = useState<string>("empty")
  const [conflict, setConflict] = useState<CoreMemoryConflict | null>(null)
  const [mergedContent, setMergedContent] = useState("")
  const [resolvingConflict, setResolvingConflict] = useState(false)
  const scopeId = scope === "agent" ? agentId : scope === "project" ? projectId : undefined

  const loadContent = useCallback(async () => {
    setLoading(true)
    try {
      const index = await getTransport().call<CoreMemoryIndex>("core_memory_get_cmd", {
        scopeType: scope,
        scopeId,
      })
      const val = index.content ?? ""
      setContent(val)
      setOriginalContent(val)
      setFileHash(index.fileHash ?? null)
      setMigrationState(index.state)
      if (index.state !== "conflict") {
        setConflict(null)
        setMergedContent("")
      }
      setLoaded(true)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::load", "Failed to load", e)
      setLoadError(coreMemoryOperationErrorToast(
        coreMemoryOperationForScope("load", scope),
        t,
        e,
      ))
    } finally {
      setLoading(false)
    }
  }, [scope, scopeId, t])

  const loadConflict = useCallback(async () => {
    try {
      const value = await getTransport().call<CoreMemoryConflict | null>(
        "core_memory_conflict_get_cmd",
        { scopeType: scope, scopeId },
      )
      setConflict(value)
      setMergedContent(value?.canonicalContent ?? "")
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::loadConflict", "Failed to load conflict", e)
      toast.error(t("settings.memoryV2.core.conflictLoadFailed"))
    }
  }, [scope, scopeId, t])

  useEffect(() => {
    loadContent()
  }, [loadContent])

  useEffect(() => {
    if (migrationState === "conflict") void loadConflict()
  }, [loadConflict, migrationState])

  useEffect(() => {
    const matches = (raw: unknown) => {
      const payload = raw as { scope?: string; scopeType?: string; scopeId?: string; agentId?: string }
      const eventScope = payload.scopeType ?? payload.scope
      if (eventScope === "all") {
        loadContent()
      } else if (eventScope === scope) {
        if (scope === "global" || payload.scopeId === scopeId || payload.agentId === scopeId) {
          loadContent()
        }
      }
    }
    const unlistenLegacy = getTransport().listen("core_memory_updated", matches)
    const unlistenV2 = getTransport().listen("memory:core_changed", matches)
    return () => {
      unlistenLegacy()
      unlistenV2()
    }
  }, [scope, scopeId, loadContent])

  const handleSave = async () => {
    setSaving(true)
    try {
      const index = await getTransport().call<CoreMemoryIndex>("core_memory_save_cmd", {
        scopeType: scope,
        scopeId,
        content,
        expectedFileHash: fileHash,
      })
      const savedContent = index.content ?? ""
      setContent(savedContent)
      setOriginalContent(savedContent)
      setFileHash(index.fileHash ?? null)
      setMigrationState(index.state)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::save", "Failed to save", e)
      setSaveStatus("failed")
      const failureToast = coreMemoryOperationErrorToast(
        coreMemoryOperationForScope("save", scope),
        t,
        e,
      )
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const hasChanges = content !== originalContent
  const resolveConflict = async (choice: "canonical" | "legacy" | "merged") => {
    if (!conflict) return
    setResolvingConflict(true)
    try {
      const index = await getTransport().call<CoreMemoryIndex>(
        "core_memory_conflict_resolve_cmd",
        {
          scopeType: scope,
          scopeId,
          resolution: {
            choice,
            expectedCanonicalHash: conflict.canonicalHash,
            expectedLegacyHash: conflict.legacyHash,
            mergedContent: choice === "merged" ? mergedContent : undefined,
          },
        },
      )
      const saved = index.content ?? ""
      setContent(saved)
      setOriginalContent(saved)
      setFileHash(index.fileHash ?? null)
      setMigrationState(index.state)
      setConflict(null)
      setMergedContent("")
      toast.success(t("settings.memoryV2.core.conflictResolved"))
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::resolveConflict", "Failed", e)
      toast.error(t("settings.memoryV2.core.conflictResolveFailed"))
      await loadContent()
    } finally {
      setResolvingConflict(false)
    }
  }
  const title = scope === "global"
    ? t("settings.coreMemoryGlobal")
    : scope === "agent"
      ? t("settings.coreMemory")
      : t("settings.memoryV2.core.projectTitle")
  const desc = scope === "global"
    ? t("settings.coreMemoryGlobalDesc")
    : scope === "agent"
      ? t("settings.coreMemoryAgentDesc")
      : t("settings.memoryV2.core.projectDesc")

  return (
    <div className="rounded-lg bg-secondary/30 mb-4 shrink-0">
      <div className="flex items-center justify-between gap-2 pr-3">
        <Button
          variant="ghost"
          onClick={() => setExpanded(!expanded)}
          className="h-auto flex-1 justify-start gap-1.5 rounded-none rounded-l-lg px-3 py-2 font-normal hover:bg-transparent"
        >
          {expanded ? <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" /> : <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />}
          <span className="text-sm font-medium">{title}</span>
          {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
          {originalContent.trim() && !expanded && (
            <span className="text-[10px] text-muted-foreground ml-1">
              ({originalContent.trim().split("\n").length} {t("settings.coreMemoryLines")})
            </span>
          )}
        </Button>
        {loaded && hasChanges && expanded && migrationState !== "conflict" && (
          <Button
            size="sm"
            className="gap-1.5 h-6 text-xs shrink-0"
            disabled={saving}
            onClick={handleSave}
            variant={saveStatus === "saved" ? "outline" : saveStatus === "failed" ? "destructive" : "default"}
          >
            {saving ? (
              <><Loader2 className="h-3 w-3 animate-spin" />{t("common.saving")}</>
            ) : saveStatus === "saved" ? (
              <><Check className="h-3 w-3" />{t("common.saved")}</>
            ) : (
              <><Save className="h-3 w-3" />{t("common.save")}</>
            )}
          </Button>
        )}
      </div>
      {expanded && (
        <div className="px-3 pb-3">
          <p className="text-xs text-muted-foreground mb-2">{desc}</p>
          {loadError && (
            <div className="mb-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
              <div className="flex items-center gap-1.5 font-medium text-foreground">
                <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
                {loadError.title}
              </div>
              {loadError.description && (
                <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
              )}
              <button
                type="button"
                className="mt-2 font-medium text-foreground underline underline-offset-2"
                onClick={() => void loadContent()}
              >
                {t("common.retry", "Retry")}
              </button>
            </div>
          )}
          {migrationState === "conflict" && (
            <div className="mb-3 space-y-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-3 text-xs">
              <div className="text-destructive">{t("settings.memoryV2.core.conflict")}</div>
              {!conflict ? (
                <Button type="button" size="sm" variant="outline" onClick={() => void loadConflict()}>
                  {t("common.retry", "Retry")}
                </Button>
              ) : (
                <>
                  <div className="grid gap-3 lg:grid-cols-2">
                    <label className="space-y-1 text-muted-foreground">
                      <span>{t("settings.memoryV2.core.conflictCanonical")}</span>
                      <Textarea value={conflict.canonicalContent} readOnly className="min-h-32 font-mono text-xs" />
                    </label>
                    <label className="space-y-1 text-muted-foreground">
                      <span>{t("settings.memoryV2.core.conflictLegacy")}</span>
                      <Textarea value={conflict.legacyContent} readOnly className="min-h-32 font-mono text-xs" />
                    </label>
                  </div>
                  <label className="block space-y-1 text-muted-foreground">
                    <span>{t("settings.memoryV2.core.conflictMerged")}</span>
                    <Textarea
                      value={mergedContent}
                      onChange={(event) => setMergedContent(event.target.value)}
                      className="min-h-36 font-mono text-xs"
                    />
                  </label>
                  <div className="flex flex-wrap justify-end gap-2">
                    <Button type="button" size="sm" variant="outline" disabled={resolvingConflict} onClick={() => void resolveConflict("canonical")}>
                      {t("settings.memoryV2.core.conflictKeepCanonical")}
                    </Button>
                    <Button type="button" size="sm" variant="outline" disabled={resolvingConflict} onClick={() => void resolveConflict("legacy")}>
                      {t("settings.memoryV2.core.conflictKeepLegacy")}
                    </Button>
                    <Button type="button" size="sm" disabled={resolvingConflict} onClick={() => void resolveConflict("merged")}>
                      {resolvingConflict && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                      {t("settings.memoryV2.core.conflictSaveMerged")}
                    </Button>
                  </div>
                </>
              )}
            </div>
          )}
          {loaded && (
            <Textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              disabled={migrationState === "conflict"}
              placeholder={t("settings.coreMemoryPlaceholder")}
              className="min-h-[80px] max-h-[200px] text-sm font-mono resize-y"
            />
          )}
        </div>
      )}
    </div>
  )
}
