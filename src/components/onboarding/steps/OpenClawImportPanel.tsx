import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertTriangle,
  Check,
  Database,
  Folder,
  Loader2,
  Server,
  Users,
  X,
} from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"

// ── Types mirroring the backend OpenClawImportPreview ──────────

type CredentialKind = "api-key-plain" | "api-key-env-ref" | "o-auth" | "token" | "missing"

interface ProviderProfilePreview {
  sourceProfileId: string
  label: string
  credentialKind: CredentialKind
  email?: string | null
  willImport: boolean
  note?: string | null
}

interface ProviderPreview {
  sourceKey: string
  suggestedName: string
  apiType: "anthropic" | "openai-chat" | "openai-responses" | "codex"
  baseUrl: string
  modelCount: number
  profiles: ProviderProfilePreview[]
  nameConflictsExisting: boolean
  apiTypeWarning?: string | null
}

interface AgentPreview {
  id: string
  name: string
  emoji?: string | null
  theme?: string | null
  avatar?: string | null
  modelInfo?: string | null
  hasSystemPrompt: boolean
  sandbox: boolean
  skillNames: string[]
  availableFiles: string[]
  alreadyExists: boolean
}

interface MemoryPreview {
  globalMdPresent: boolean
  /**
   * Tuple-encoded as ["agentId", count]. serde serializes Rust tuples this way.
   */
  agentMdCounts: [string, number][]
}

interface ImportPreview {
  stateDir: string
  stateDirPresent: boolean
  providers: ProviderPreview[]
  agents: AgentPreview[]
  memories: MemoryPreview
  warnings: string[]
}

interface ImportAgentRequest {
  sourceId: string
  targetId: string
  name: string
  emoji?: string | null
  vibe?: string | null
  sandbox: boolean
  importFiles: string[]
}

interface ImportRequestPayload {
  importProviderKeys: string[]
  importAgents: ImportAgentRequest[]
  importGlobalMemory: boolean
  importAgentMemories: string[]
}

interface ImportSummary {
  providersAdded: string[]
  agents: Array<{
    sourceId: string
    importedId: string
    name: string
    success: boolean
    error?: string | null
  }>
  memoriesAdded: number
  warnings: string[]
}

// ── Component ──────────────────────────────────────────────────

interface OpenClawImportPanelProps {
  /** Called when user opts to skip without importing. */
  onSkip: () => void
  /** Called after a successful import (summary attached). */
  onImported: (summary: ImportSummary) => void
  /**
   * Suppress the "skip" path button (used when the panel is embedded in a
   * settings dialog where the surrounding chrome already has a Close button).
   */
  hideSkip?: boolean
}

export function OpenClawImportPanel({
  onSkip,
  onImported,
  hideSkip,
}: OpenClawImportPanelProps) {
  const { t } = useTranslation()
  const [scanning, setScanning] = useState(true)
  const [scanError, setScanError] = useState<string | null>(null)
  const [preview, setPreview] = useState<ImportPreview | null>(null)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [importing, setImporting] = useState(false)
  const [lastSummary, setLastSummary] = useState<ImportSummary | null>(null)

  const [selectedProviders, setSelectedProviders] = useState<Set<string>>(new Set())
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set())
  const [agentEdits, setAgentEdits] = useState<Record<string, ImportAgentRequest>>({})
  const [globalMemory, setGlobalMemory] = useState(false)
  const [selectedAgentMemories, setSelectedAgentMemories] = useState<Set<string>>(new Set())

  // ── Initial scan ──
  useEffect(() => {
    let cancelled = false
    const run = async () => {
      setScanning(true)
      try {
        const result = await getTransport().call<ImportPreview>("scan_openclaw_full")
        if (cancelled) return
        setPreview(result)
        const defaultAgentIds = new Set(
          result.agents.filter((a) => !a.alreadyExists).map((a) => a.id),
        )
        setSelectedProviders(new Set(result.providers.map((p) => p.sourceKey)))
        setSelectedAgents(defaultAgentIds)
        setGlobalMemory(result.memories.globalMdPresent)
        setSelectedAgentMemories(
          new Set(
            result.memories.agentMdCounts
              .map((entry) => entry[0])
              .filter((agentId) => defaultAgentIds.has(agentId)),
          ),
        )
        const seeded: Record<string, ImportAgentRequest> = {}
        for (const a of result.agents) {
          seeded[a.id] = {
            sourceId: a.id,
            targetId: a.alreadyExists ? `${a.id}-imported` : a.id,
            name: a.name,
            emoji: a.emoji ?? null,
            vibe: null,
            sandbox: a.sandbox,
            importFiles: a.availableFiles,
          }
        }
        setAgentEdits(seeded)
      } catch (e) {
        logger.warn("onboarding", "OpenClawImportPanel", "scan_openclaw_full failed", e)
        if (!cancelled) setScanError(String(e))
      } finally {
        if (!cancelled) setScanning(false)
      }
    }
    void run()
    return () => {
      cancelled = true
    }
  }, [])

  const toggle = <T,>(set: Set<T>, value: T): Set<T> => {
    const next = new Set(set)
    if (next.has(value)) next.delete(value)
    else next.add(value)
    return next
  }

  const agentIdsWithMemory = useMemo(() => {
    return new Set(preview?.memories.agentMdCounts.map((entry) => entry[0]) ?? [])
  }, [preview])

  const toggleAgent = (agentId: string) => {
    const willSelect = !selectedAgents.has(agentId)
    setSelectedAgents((prev) => {
      const next = new Set(prev)
      if (willSelect) next.add(agentId)
      else next.delete(agentId)
      return next
    })
    setSelectedAgentMemories((prev) => {
      const next = new Set(prev)
      if (willSelect && agentIdsWithMemory.has(agentId)) next.add(agentId)
      else next.delete(agentId)
      return next
    })
    setLastSummary(null)
    setSaveStatus("idle")
  }

  const selectedAgentMemoryCount = useMemo(() => {
    let count = 0
    for (const agentId of selectedAgentMemories) {
      if (selectedAgents.has(agentId)) count += 1
    }
    return count
  }, [selectedAgentMemories, selectedAgents])

  const totalSelected = useMemo(() => {
    return (
      selectedProviders.size +
      selectedAgents.size +
      (globalMemory ? 1 : 0) +
      selectedAgentMemoryCount
    )
  }, [selectedProviders, selectedAgents, globalMemory, selectedAgentMemoryCount])

  async function handleImport() {
    if (!preview) return
    setImporting(true)
    setSaveStatus("idle")
    setLastSummary(null)
    try {
      const importAgents: ImportAgentRequest[] = preview.agents
        .filter((a) => selectedAgents.has(a.id))
        .map((a) => agentEdits[a.id] ?? {
          sourceId: a.id,
          targetId: a.id,
          name: a.name,
          emoji: a.emoji ?? null,
          vibe: null,
          sandbox: a.sandbox,
          importFiles: a.availableFiles,
        })

      // Translate "selected agent ids (source)" to target ids since the
      // backend keys memory imports off the canonical target.
      const selectedAgentTargetIds = new Set(
        importAgents
          .filter((req) => selectedAgentMemories.has(req.sourceId))
          .map((req) => req.targetId),
      )

      const payload: ImportRequestPayload = {
        importProviderKeys: Array.from(selectedProviders),
        importAgents,
        importGlobalMemory: globalMemory,
        importAgentMemories: Array.from(selectedAgentTargetIds),
      }
      const summary = await getTransport().call<ImportSummary>(
        "import_openclaw_full",
        { request: payload },
      )
      setSaveStatus("saved")
      setLastSummary(summary)
      const failedAgents = summary.agents.filter((a) => !a.success)
      if (summary.providersAdded.length > 0) {
        setSelectedProviders(new Set())
      }
      if (summary.memoriesAdded > 0) {
        setGlobalMemory(false)
      }
      if (failedAgents.length > 0) {
        const failedSourceIds = new Set(failedAgents.map((a) => a.sourceId))
        setSelectedAgents((prev) =>
          new Set(Array.from(prev).filter((agentId) => failedSourceIds.has(agentId))),
        )
        setSelectedAgentMemories((prev) =>
          new Set(Array.from(prev).filter((agentId) => failedSourceIds.has(agentId))),
        )
      }
      if (failedAgents.length > 0 || summary.warnings.length > 0) {
        setSaveStatus(failedAgents.length > 0 ? "failed" : "saved")
        return
      }
      onImported(summary)
    } catch (e) {
      logger.error("onboarding", "OpenClawImportPanel", "import_openclaw_full failed", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setImporting(false)
    }
  }

  // ── Loading / not-detected branches ──
  if (scanning) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        <p className="text-sm text-muted-foreground">
          {t("onboarding.importOpenClaw.scanning")}
        </p>
      </div>
    )
  }
  if (scanError) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
        <AlertTriangle className="h-6 w-6 text-amber-600" />
        <p className="text-sm text-muted-foreground">
          {t("onboarding.importOpenClaw.scanFailed")}
        </p>
        <p className="text-xs text-muted-foreground/70 max-w-md break-all">{scanError}</p>
        {!hideSkip && (
          <Button variant="ghost" onClick={onSkip} className="mt-2">
            {t("onboarding.importOpenClaw.skip")}
          </Button>
        )}
      </div>
    )
  }
  if (!preview || !preview.stateDirPresent) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
        <Folder className="h-6 w-6 text-muted-foreground" />
        <p className="text-sm text-muted-foreground max-w-md">
          {t("onboarding.importOpenClaw.notFound")}
        </p>
        {preview?.stateDir && (
          <code className="text-[11px] text-muted-foreground/60 px-2 py-1 rounded bg-muted">
            {preview.stateDir}
          </code>
        )}
        {!hideSkip && (
          <Button variant="default" onClick={onSkip} className="mt-2">
            {t("onboarding.importOpenClaw.continue")}
          </Button>
        )}
      </div>
    )
  }

  const hasAnything =
    preview.providers.length > 0 ||
    preview.agents.length > 0 ||
    preview.memories.globalMdPresent ||
    preview.memories.agentMdCounts.length > 0

  if (!hasAnything) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
        <Folder className="h-6 w-6 text-muted-foreground" />
        <p className="text-sm text-muted-foreground max-w-md">
          {t("onboarding.importOpenClaw.empty")}
        </p>
        <code className="text-[11px] text-muted-foreground/60 px-2 py-1 rounded bg-muted">
          {preview.stateDir}
        </code>
        {!hideSkip && (
          <Button variant="default" onClick={onSkip} className="mt-2">
            {t("onboarding.importOpenClaw.continue")}
          </Button>
        )}
      </div>
    )
  }

  return (
    <div className="px-2 sm:px-4 space-y-6">
      <div className="flex flex-col gap-1">
        <h2 className="text-xl font-semibold tracking-tight">
          {t("onboarding.importOpenClaw.title")}
        </h2>
        <p className="text-sm text-muted-foreground">
          {t("onboarding.importOpenClaw.subtitle")}{" "}
          <code className="text-[11px] px-1 rounded bg-muted">{preview.stateDir}</code>
        </p>
      </div>

      {/* Providers */}
      {preview.providers.length > 0 && (
        <SectionCard
          icon={<Server className="h-4 w-4" />}
          title={t("onboarding.importOpenClaw.providers.title")}
          count={selectedProviders.size}
          total={preview.providers.length}
          onToggleAll={() => {
            if (selectedProviders.size === preview.providers.length) {
              setSelectedProviders(new Set())
            } else {
              setSelectedProviders(
                new Set(preview.providers.map((p) => p.sourceKey)),
              )
            }
            setLastSummary(null)
            setSaveStatus("idle")
          }}
        >
          <div className="space-y-2">
            {preview.providers.map((p) => {
              const checked = selectedProviders.has(p.sourceKey)
              const importableProfiles = p.profiles.filter((pp) => pp.willImport)
              return (
                <div
                  key={p.sourceKey}
                  className={cn(
                    "rounded-md border px-3 py-2.5 cursor-pointer hover:bg-accent/40 transition",
                    checked && "border-primary/60 bg-primary/5",
                  )}
                  onClick={() => {
                    setSelectedProviders((prev) => toggle(prev, p.sourceKey))
                    setLastSummary(null)
                    setSaveStatus("idle")
                  }}
                >
                  <div className="flex items-start gap-3">
                    <Checkbox checked={checked} />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-baseline gap-2 flex-wrap">
                        <span className="font-medium text-sm">{p.suggestedName}</span>
                        <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
                          {p.apiType}
                        </span>
                        {p.nameConflictsExisting && (
                          <span className="text-[10px] text-amber-600">
                            {t("onboarding.importOpenClaw.providers.nameConflict")}
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground truncate">
                        {p.baseUrl}
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1">
                        <Chip>
                          {t("onboarding.importOpenClaw.providers.modelCount", {
                            count: p.modelCount,
                          })}
                        </Chip>
                        <Chip variant="success">
                          {t("onboarding.importOpenClaw.providers.profileWithPlainKey", {
                            count: importableProfiles.length,
                          })}
                        </Chip>
                        {p.profiles.some((pp) => pp.credentialKind === "o-auth") && (
                          <Chip variant="warn">
                            {t("onboarding.importOpenClaw.providers.oauthExcluded")}
                          </Chip>
                        )}
                      </div>
                      {p.apiTypeWarning && (
                        <div className="mt-1 text-[11px] text-amber-700/80">
                          {p.apiTypeWarning}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        </SectionCard>
      )}

      {/* Agents */}
      {preview.agents.length > 0 && (
        <SectionCard
          icon={<Users className="h-4 w-4" />}
          title={t("onboarding.importOpenClaw.agents.title")}
          count={selectedAgents.size}
          total={preview.agents.length}
          onToggleAll={() => {
            if (selectedAgents.size === preview.agents.length) {
              setSelectedAgents(new Set())
              setSelectedAgentMemories(new Set())
            } else {
              setSelectedAgents(new Set(preview.agents.map((a) => a.id)))
              setSelectedAgentMemories(new Set(preview.memories.agentMdCounts.map((m) => m[0])))
            }
            setLastSummary(null)
            setSaveStatus("idle")
          }}
        >
          <div className="space-y-2">
            {preview.agents.map((a) => {
              const checked = selectedAgents.has(a.id)
              const edit = agentEdits[a.id]
              return (
                <div
                  key={a.id}
                  className={cn(
                    "rounded-md border px-3 py-2.5",
                    checked && "border-primary/60 bg-primary/5",
                  )}
                >
                  <div className="flex items-start gap-3">
                    <button
                      type="button"
                      className="mt-0.5"
                      onClick={() => toggleAgent(a.id)}
                    >
                      <Checkbox checked={checked} />
                    </button>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-baseline gap-2 flex-wrap">
                        <span className="font-medium text-sm">
                          {a.emoji ? `${a.emoji} ` : ""}
                          {a.name}
                        </span>
                        {a.alreadyExists && (
                          <span className="text-[10px] text-amber-600">
                            {t("onboarding.importOpenClaw.agents.alreadyExists")}
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        ID:{" "}
                        <input
                          type="text"
                          value={edit?.targetId ?? a.id}
                          onChange={(ev) =>
                            setAgentEdits((prev) => ({
                              ...prev,
                              [a.id]: { ...edit!, targetId: ev.target.value },
                            }))
                          }
                          className="bg-transparent border-b border-muted-foreground/30 outline-none font-mono text-xs px-1 focus:border-primary"
                        />
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {a.modelInfo && <Chip>{a.modelInfo}</Chip>}
                        {a.hasSystemPrompt && (
                          <Chip>
                            {t("onboarding.importOpenClaw.agents.hasSystemPrompt")}
                          </Chip>
                        )}
                        {a.sandbox && (
                          <Chip variant="warn">
                            {t("onboarding.importOpenClaw.agents.sandbox")}
                          </Chip>
                        )}
                        {a.skillNames.length > 0 && (
                          <Chip>
                            {t("onboarding.importOpenClaw.agents.skills", {
                              count: a.skillNames.length,
                            })}
                          </Chip>
                        )}
                        {a.availableFiles.map((f) => (
                          <Chip key={f}>{f}</Chip>
                        ))}
                      </div>
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        </SectionCard>
      )}

      {/* Memory */}
      {(preview.memories.globalMdPresent || preview.memories.agentMdCounts.length > 0) && (
        <SectionCard
          icon={<Database className="h-4 w-4" />}
          title={t("onboarding.importOpenClaw.memory.title")}
        >
          <div className="space-y-2">
            {preview.memories.globalMdPresent && (
              <label className="flex items-center gap-3 px-3 py-2 rounded-md border cursor-pointer hover:bg-accent/40">
                <Checkbox
                  checked={globalMemory}
                  onClick={(e) => {
                    e.stopPropagation()
                    setLastSummary(null)
                    setSaveStatus("idle")
                    setGlobalMemory((v) => !v)
                  }}
                />
                <span className="text-sm">
                  {t("onboarding.importOpenClaw.memory.global")}
                </span>
              </label>
            )}
            {preview.memories.agentMdCounts.map((entry) => {
              const [agentId, count] = entry
              const agentSelected = selectedAgents.has(agentId)
              const checked = agentSelected && selectedAgentMemories.has(agentId)
              return (
                <label
                  key={agentId}
                  className={cn(
                    "flex items-center gap-3 px-3 py-2 rounded-md border cursor-pointer hover:bg-accent/40",
                    !agentSelected && "opacity-50 cursor-not-allowed hover:bg-transparent",
                  )}
                >
                  <Checkbox
                    checked={checked}
                    disabled={!agentSelected}
                    onClick={(e) => {
                      e.stopPropagation()
                      if (!agentSelected) return
                      setSelectedAgentMemories((prev) => toggle(prev, agentId))
                      setLastSummary(null)
                      setSaveStatus("idle")
                    }}
                  />
                  <span className="text-sm flex-1">
                    {t("onboarding.importOpenClaw.memory.perAgent", {
                      agent: agentId,
                      count,
                    })}
                  </span>
                </label>
              )
            })}
          </div>
        </SectionCard>
      )}

      {/* Warnings */}
      {preview.warnings.length > 0 && (
        <div className="rounded-md border border-amber-300/40 bg-amber-50 dark:bg-amber-950/30 px-3 py-2.5 space-y-1">
          <div className="flex items-center gap-1.5 text-xs font-medium text-amber-800 dark:text-amber-200">
            <AlertTriangle className="h-3.5 w-3.5" />
            {t("onboarding.importOpenClaw.warningsTitle")}
          </div>
          <ul className="list-disc list-inside text-[11px] text-amber-900/90 dark:text-amber-100/90 space-y-0.5">
            {preview.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}

      {lastSummary && <ImportSummaryNotice summary={lastSummary} />}

      <ImportActionRow
        importing={importing}
        saveStatus={saveStatus}
        totalSelected={totalSelected}
        onSkip={onSkip}
        onImport={handleImport}
        onContinue={() => {
          if (lastSummary) onImported(lastSummary)
        }}
        canContinue={Boolean(
          lastSummary &&
            lastSummary.agents.every((a) => a.success) &&
            lastSummary.warnings.length > 0,
        )}
        hideSkip={hideSkip}
      />
    </div>
  )
}

interface ImportActionRowProps {
  importing: boolean
  saveStatus: "idle" | "saved" | "failed"
  totalSelected: number
  onSkip: () => void
  onImport: () => void
  onContinue: () => void
  canContinue: boolean
  hideSkip?: boolean
}

function ImportActionRow({
  importing,
  saveStatus,
  totalSelected,
  onSkip,
  onImport,
  onContinue,
  canContinue,
  hideSkip,
}: ImportActionRowProps) {
  const { t } = useTranslation()
  const config: Record<
    "idle" | "saved" | "failed",
    { className?: string; icon?: typeof Check; labelKey: string }
  > = {
    idle: { labelKey: "onboarding.importOpenClaw.import.button" },
    saved: {
      className: "bg-emerald-600 hover:bg-emerald-700",
      icon: Check,
      labelKey: "onboarding.importOpenClaw.success",
    },
    failed: {
      className: "bg-red-600 hover:bg-red-700",
      icon: X,
      labelKey: "onboarding.importOpenClaw.failed",
    },
  }
  const current = config[saveStatus]
  const Icon = importing ? Loader2 : current.icon
  const buttonLabel = canContinue
    ? t("onboarding.importOpenClaw.continue")
    : saveStatus === "failed"
      ? t("project.question.retry")
      : saveStatus === "idle"
        ? t(current.labelKey, { n: totalSelected })
        : t(current.labelKey)
  return (
    <div className="flex items-center gap-3 justify-end">
      {!hideSkip && (
        <Button variant="ghost" onClick={onSkip} disabled={importing}>
          {t("onboarding.importOpenClaw.skip")}
        </Button>
      )}
      <Button
        onClick={canContinue ? onContinue : onImport}
        disabled={importing || (!canContinue && totalSelected === 0)}
        className={cn(current.className)}
      >
        {Icon && <Icon className={cn("h-4 w-4 mr-2", importing && "animate-spin")} />}
        {buttonLabel}
      </Button>
    </div>
  )
}

function ImportSummaryNotice({ summary }: { summary: ImportSummary }) {
  const { t } = useTranslation()
  const failedAgents = summary.agents.filter((a) => !a.success)
  const successCount = summary.agents.length - failedAgents.length
  const hasProblems = failedAgents.length > 0 || summary.warnings.length > 0
  return (
    <div
      className={cn(
        "rounded-md border px-3 py-2.5 space-y-2 text-sm",
        hasProblems
          ? "border-amber-300/50 bg-amber-50 dark:bg-amber-950/30"
          : "border-emerald-300/50 bg-emerald-50 dark:bg-emerald-950/30",
      )}
    >
      <div className="flex items-center gap-1.5 font-medium">
        {hasProblems ? (
          <AlertTriangle className="h-4 w-4 text-amber-700 dark:text-amber-200" />
        ) : (
          <Check className="h-4 w-4 text-emerald-700 dark:text-emerald-200" />
        )}
        <span>
          {failedAgents.length > 0
            ? t("settings.openclawImportPartial", {
                success: successCount,
                total: summary.agents.length,
              })
            : summary.warnings.length > 0
              ? t("onboarding.importOpenClaw.partial")
              : t("onboarding.importOpenClaw.success")}
        </span>
      </div>

      {failedAgents.length > 0 && (
        <div className="space-y-1">
          {failedAgents.map((agent) => (
            <div key={agent.sourceId} className="text-xs text-destructive break-words">
              {agent.name}: {agent.error ?? t("onboarding.importOpenClaw.failed")}
            </div>
          ))}
        </div>
      )}

      {summary.providersAdded.length > 0 && (
        <div className="text-xs text-muted-foreground">
          {t("onboarding.importOpenClaw.providers.title")}: {summary.providersAdded.length}
        </div>
      )}
      {summary.memoriesAdded > 0 && (
        <div className="text-xs text-muted-foreground">
          {t("onboarding.importOpenClaw.memory.title")}: {summary.memoriesAdded}
        </div>
      )}

      {summary.warnings.length > 0 && (
        <ul className="list-disc list-inside text-xs text-amber-900/90 dark:text-amber-100/90 space-y-0.5">
          {summary.warnings.map((warning, index) => (
            <li key={index}>{warning}</li>
          ))}
        </ul>
      )}
    </div>
  )
}

// ── Tiny inline UI helpers (keep self-contained, avoid pulling new shadcn deps) ──

function SectionCard({
  icon,
  title,
  count,
  total,
  onToggleAll,
  children,
}: {
  icon: React.ReactNode
  title: string
  count?: number
  total?: number
  onToggleAll?: () => void
  children: React.ReactNode
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-lg border bg-card p-4 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm font-medium">
          {icon}
          <span>{title}</span>
          {count !== undefined && total !== undefined && (
            <span className="text-xs text-muted-foreground">
              ({count}/{total})
            </span>
          )}
        </div>
        {onToggleAll && total !== undefined && (
          <Button variant="ghost" size="sm" onClick={onToggleAll} className="h-7">
            {count === total
              ? t("onboarding.importOpenClaw.deselectAll")
              : t("onboarding.importOpenClaw.selectAll")}
          </Button>
        )}
      </div>
      {children}
    </div>
  )
}

function Checkbox({
  checked,
  onClick,
  disabled,
}: {
  checked: boolean
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "h-4 w-4 rounded border flex items-center justify-center shrink-0 mt-0.5",
        checked
          ? "bg-primary border-primary text-primary-foreground"
          : "border-muted-foreground/40 bg-background",
        disabled && "cursor-not-allowed",
      )}
    >
      {checked && <Check className="h-3 w-3" />}
    </button>
  )
}

function Chip({
  children,
  variant,
}: {
  children: React.ReactNode
  variant?: "warn" | "success"
}) {
  return (
    <span
      className={cn(
        "text-[10px] px-1.5 py-0.5 rounded border tabular-nums",
        variant === "warn" && "border-amber-300/60 bg-amber-50 text-amber-800",
        variant === "success" && "border-emerald-300/60 bg-emerald-50 text-emerald-800",
        !variant && "border-muted-foreground/20 bg-muted/40 text-muted-foreground",
      )}
    >
      {children}
    </span>
  )
}

export type { ImportSummary as OpenClawImportSummary }
