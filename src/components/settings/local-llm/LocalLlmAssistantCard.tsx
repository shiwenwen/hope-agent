import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Cpu,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  Sparkles,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { withEventListener } from "@/lib/transport-events"
import { logger } from "@/lib/logger"
import { formatBytesFromMb, formatGbFromMb } from "@/lib/format"
import { cn } from "@/lib/utils"
import {
  InstallProgressDialog,
  type ProgressFrame,
} from "@/components/settings/local-llm/InstallProgressDialog"
import { IconTip } from "@/components/ui/tooltip"

// ── Wire types (mirror ha_core::local_llm::types) ─────────────────

type BudgetSource = "unified-memory" | "dedicated-vram" | "system-memory"
type OllamaPhase = "not-installed" | "installed" | "running"
type RecommendationReason = "insufficient" | "unified-memory" | "dgpu" | "ram-fallback"
type InstallProgressKind = "step" | "log" | "error"

interface GpuInfo {
  name: string
  vramMb?: number | null
}

interface HardwareInfo {
  os: string
  totalMemoryMb: number
  availableMemoryMb: number
  gpu?: GpuInfo | null
  budgetSource: BudgetSource
  budgetMb: number
}

interface ModelCandidate {
  id: string
  displayName: string
  family: string
  sizeMb: number
  contextWindow: number
  reasoning: boolean
}

interface ModelRecommendation {
  hardware: HardwareInfo
  recommended: ModelCandidate | null
  alternatives: ModelCandidate[]
  reason: RecommendationReason
}

interface OllamaStatus {
  phase: OllamaPhase
  baseUrl: string
  installScriptSupported: boolean
}

interface PullProgressPayload {
  modelId: string
  phase: string
  percent?: number | null
}

interface InstallProgressPayload {
  kind: InstallProgressKind
  message: string
}

interface DesktopOpenResult {
  ok?: boolean
}

// ── Helpers ───────────────────────────────────────────────────────

const EVENT_LOCAL_LLM_INSTALL_PROGRESS = "local_llm:install_progress"
const EVENT_LOCAL_LLM_PULL_PROGRESS = "local_llm:pull_progress"
const MAX_DIALOG_LOG_LINES = 240

const PHASE_KEY: Record<string, string> = {
  starting: "settings.localLlm.phases.starting",
  "pulling manifest": "settings.localLlm.phases.pullingManifest",
  downloading: "settings.localLlm.phases.downloading",
  "verifying digest": "settings.localLlm.phases.verifying",
  "writing manifest": "settings.localLlm.phases.writingManifest",
  success: "settings.localLlm.phases.success",
  "register-provider": "settings.localLlm.phases.registerProvider",
  done: "settings.localLlm.phases.done",
}

function formatLogLine(message: string): string {
  return `[${new Date().toLocaleTimeString()}] ${message}`
}

function reasonText(
  rec: ModelRecommendation,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const hw = rec.hardware
  switch (rec.reason) {
    case "insufficient":
      return t("settings.localLlm.hardware.insufficient")
    case "unified-memory":
      return t("settings.localLlm.hardware.macOs", {
        memory: formatGbFromMb(hw.totalMemoryMb),
        budget: formatGbFromMb(hw.budgetMb),
      })
    case "dgpu":
      return t("settings.localLlm.hardware.dgpu", {
        gpu: hw.gpu?.name ?? "GPU",
        vram: hw.gpu?.vramMb ? formatGbFromMb(hw.gpu.vramMb) : "?",
        budget: formatGbFromMb(hw.budgetMb),
      })
    default:
      return t("settings.localLlm.hardware.ramFallback", {
        memory: formatGbFromMb(hw.totalMemoryMb),
        budget: formatGbFromMb(hw.budgetMb),
      })
  }
}

// ── Component ─────────────────────────────────────────────────────

export default function LocalLlmAssistantCard({
  onProviderInstalled,
  compact = false,
}: {
  onProviderInstalled: () => void
  compact?: boolean
}) {
  const { t } = useTranslation()
  const [recommendation, setRecommendation] = useState<ModelRecommendation | null>(null)
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [busy, setBusy] = useState<null | "install" | "start" | "pull">(null)
  const [showAlternatives, setShowAlternatives] = useState(false)
  // `null` = follow recommendation; non-null = user explicitly picked.
  const [chosen, setChosen] = useState<ModelCandidate | null>(null)
  const [error, setError] = useState<string | null>(null)

  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogTitle, setDialogTitle] = useState("")
  const [dialogSubtitle, setDialogSubtitle] = useState<string | undefined>(undefined)
  const [dialogFrame, setDialogFrame] = useState<ProgressFrame | null>(null)
  const [dialogLogs, setDialogLogs] = useState<string[]>([])
  const [dialogDone, setDialogDone] = useState(false)
  const [dialogError, setDialogError] = useState<string | null>(null)

  const appendDialogLog = useCallback((message: string) => {
    const trimmed = message.trim()
    if (!trimmed) return
    const line = formatLogLine(trimmed)
    setDialogLogs((prev) => {
      if (prev[prev.length - 1] === line) return prev
      return [...prev.slice(-(MAX_DIALOG_LOG_LINES - 1)), line]
    })
  }, [])

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      const [rec, status] = await Promise.all([
        getTransport().call<ModelRecommendation>("local_llm_recommend_model"),
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
      ])
      setRecommendation(rec)
      setOllama(status)
    } catch (e) {
      logger.error("local-llm", "refresh", "Failed to detect hardware/ollama", e)
      setError(String(e))
    } finally {
      setRefreshing(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const phaseLabel = useCallback(
    (phase: string | undefined) => {
      if (!phase) return ""
      const key = PHASE_KEY[phase.toLowerCase()]
      return key ? t(key) : phase
    },
    [t],
  )

  const startOllama = useCallback(async () => {
    setBusy("start")
    setError(null)
    try {
      await getTransport().call("local_llm_start_ollama")
      await refresh()
    } catch (e) {
      setError(t("settings.localLlm.error.startFailed", { message: String(e) }))
    } finally {
      setBusy(null)
    }
  }, [refresh, t])

  const installOllama = useCallback(async () => {
    setBusy("install")
    setError(null)
    setDialogOpen(true)
    setDialogTitle(t("settings.localLlm.install.title"))
    setDialogSubtitle(undefined)
    setDialogFrame({ phase: "starting", message: t("settings.localLlm.phases.starting") })
    setDialogLogs([])
    setDialogDone(false)
    setDialogError(null)

    const handleProgress = (raw: unknown) => {
      const p = parsePayload<InstallProgressPayload>(raw)
      if (p.kind === "step") {
        setDialogFrame({ phase: p.message, message: p.message })
        appendDialogLog(p.message)
      } else if (p.kind === "log") {
        appendDialogLog(p.message)
      } else if (p.kind === "error") {
        setDialogError(p.message)
        appendDialogLog(p.message)
      }
    }

    let installed = false
    try {
      await withEventListener(EVENT_LOCAL_LLM_INSTALL_PROGRESS, handleProgress, async () => {
        await getTransport().call("local_llm_install_ollama")
        installed = true
        setDialogFrame({
          phase: "starting",
          message: t("settings.localLlm.buttons.startOllama"),
        })
        appendDialogLog(t("settings.localLlm.buttons.startOllama"))
        await getTransport().call("local_llm_start_ollama")
      })
      setDialogDone(true)
      appendDialogLog(t("settings.localLlm.phases.done"))
      await refresh()
      setTimeout(() => setDialogOpen(false), 800)
    } catch (e) {
      const msg = String(e)
      setDialogError(msg)
      appendDialogLog(msg)
      setError(
        t(
          installed
            ? "settings.localLlm.error.startFailed"
            : "settings.localLlm.error.installFailed",
          {
            message: msg,
          },
        ),
      )
    } finally {
      setBusy(null)
    }
  }, [appendDialogLog, refresh, t])

  const installModel = useCallback(
    async (model: ModelCandidate) => {
      setBusy("pull")
      setError(null)
      setDialogOpen(true)
      setDialogTitle(t("settings.localLlm.buttons.installModel", { model: model.displayName }))
      setDialogSubtitle(model.id)
      setDialogFrame({
        phase: "starting",
        message: t("settings.localLlm.phases.starting"),
        percent: null,
      })
      setDialogLogs([])
      setDialogDone(false)
      setDialogError(null)

      const handleProgress = (raw: unknown) => {
        const p = parsePayload<PullProgressPayload>(raw)
        const label = phaseLabel(p.phase) || p.phase
        const progressSuffix = p.percent == null ? "" : ` ${Math.round(p.percent)}%`
        setDialogFrame({
          phase: p.phase,
          message: label,
          percent: p.percent ?? null,
        })
        appendDialogLog(`${label}${progressSuffix}`)
      }

      try {
        await withEventListener(EVENT_LOCAL_LLM_PULL_PROGRESS, handleProgress, () =>
          getTransport().call("local_llm_pull_and_activate", { model }),
        )
        setDialogDone(true)
        appendDialogLog(t("settings.localLlm.phases.done"))
        // Hold the 100% / checkmark frame briefly so users register the
        // success state before we reload.
        setTimeout(() => {
          setDialogOpen(false)
          onProviderInstalled()
        }, 800)
      } catch (e) {
        const msg = String(e)
        setDialogError(msg)
        appendDialogLog(msg)
        setError(t("settings.localLlm.error.pullFailed", { message: msg }))
      } finally {
        setBusy(null)
      }
    },
    [appendDialogLog, onProviderInstalled, phaseLabel, t],
  )

  const openDownloadPage = useCallback(() => {
    const url = "https://ollama.com/download"
    const openInBrowser = () => window.open(url, "_blank", "noopener")
    void getTransport()
      .call<DesktopOpenResult | void>("open_url", { url })
      .then((result) => {
        if (result && typeof result === "object" && result.ok === false) {
          openInBrowser()
        }
      })
      .catch(openInBrowser)
  }, [])

  if (!recommendation) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-card/40 p-4">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.localLlm.detecting")}
        </div>
      </div>
    )
  }

  const recommended = chosen ?? recommendation.recommended
  const insufficient = !recommended
  const actionButtonClassName = compact
    ? "h-auto min-h-8 px-2.5 py-1.5 text-xs whitespace-normal"
    : undefined

  // Decide which primary action is exposed.
  const renderAction = () => {
    if (insufficient || !ollama) return null

    if (ollama.phase === "not-installed") {
      if (!ollama.installScriptSupported) {
        return (
          <Button
            variant="default"
            size="sm"
            className={actionButtonClassName}
            onClick={openDownloadPage}
          >
            <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
            {t("settings.localLlm.buttons.downloadOllama")}
          </Button>
        )
      }
      return (
        <Button
          variant="default"
          size="sm"
          className={actionButtonClassName}
          onClick={() => void installOllama()}
          disabled={busy !== null}
        >
          {busy === "install" ? (
            <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
          ) : (
            <Download className="h-3.5 w-3.5 mr-1.5" />
          )}
          {t("settings.localLlm.buttons.installOllama")}
        </Button>
      )
    }

    if (ollama.phase === "installed") {
      return (
        <Button
          variant="default"
          size="sm"
          className={actionButtonClassName}
          onClick={() => void startOllama()}
          disabled={busy !== null}
        >
          {busy === "start" ? (
            <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5 mr-1.5" />
          )}
          {t("settings.localLlm.buttons.startOllama")}
        </Button>
      )
    }

    return (
      <Button
        variant="default"
        size="sm"
        className={actionButtonClassName}
        onClick={() => recommended && void installModel(recommended)}
        disabled={busy !== null || !recommended}
      >
        {busy === "pull" ? (
          <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
        ) : (
          <Download className="h-3.5 w-3.5 mr-1.5" />
        )}
        {t("settings.localLlm.buttons.installModel", {
          model: recommended?.displayName ?? "",
        })}
      </Button>
    )
  }

  return (
    <>
      <div
        className={cn(
          "rounded-xl border border-primary/30 bg-gradient-to-br from-primary/5 to-card",
          compact ? "p-3 space-y-2" : "p-4 space-y-3",
        )}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-3 min-w-0">
            <div
              className={cn(
                "rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0",
                compact ? "w-8 h-8" : "w-9 h-9",
              )}
            >
              <Sparkles className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-foreground">
                {t("settings.localLlm.title")}
              </div>
              {compact ? (
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground mt-0.5 min-w-0">
                  <Cpu className="h-3 w-3 shrink-0" />
                  <span className="truncate">{reasonText(recommendation, t)}</span>
                </div>
              ) : (
                <div className="text-[11px] text-muted-foreground mt-0.5">
                  {t("settings.localLlm.subtitle")}
                </div>
              )}
            </div>
          </div>
          <IconTip label={t("common.refresh")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0"
              onClick={() => void refresh()}
              disabled={refreshing}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${refreshing ? "animate-spin" : ""}`} />
            </Button>
          </IconTip>
        </div>

        {!compact && (
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <Cpu className="h-3 w-3" />
            <span className="truncate">{reasonText(recommendation, t)}</span>
          </div>
        )}

        {recommended ? (
          <div
            className={cn("rounded-lg border border-border/60 bg-card", compact ? "p-2.5" : "p-3")}
          >
            <div
              className={cn(
                "gap-3",
                compact
                  ? "flex flex-col sm:flex-row sm:items-center sm:justify-between"
                  : "flex items-start justify-between",
              )}
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-foreground">
                    {recommended.displayName}
                  </span>
                  <span className="text-[10px] uppercase tracking-wide text-emerald-700 dark:text-emerald-300 bg-emerald-500/10 border border-emerald-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localLlm.recommended")}
                  </span>
                </div>
                <div className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1.5 flex-wrap">
                  <span>{formatBytesFromMb(recommended.sizeMb)}</span>
                  <span>·</span>
                  <span>
                    {t("settings.localLlm.contextWindow", {
                      n: recommended.contextWindow.toLocaleString(),
                    })}
                  </span>
                  {recommended.reasoning && (
                    <>
                      <span>·</span>
                      <span className="text-amber-600 dark:text-amber-400">
                        {t("settings.localLlm.reasoning")}
                      </span>
                    </>
                  )}
                  <span>·</span>
                  <span
                    className={cn(
                      "font-mono text-[10px] text-muted-foreground/70",
                      compact && "hidden sm:inline",
                    )}
                  >
                    {recommended.id}
                  </span>
                </div>
              </div>
              {compact ? (
                <div className="shrink-0 self-start sm:self-auto">{renderAction()}</div>
              ) : (
                ollama?.phase === "running" && (
                  <span className="text-emerald-600 dark:text-emerald-400 flex items-center gap-1 text-[11px] shrink-0">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    {t("settings.localLlm.ready")}
                  </span>
                )
              )}
            </div>

            {recommendation.alternatives.length > 1 && (
              <button
                type="button"
                className="mt-2 text-[11px] text-muted-foreground hover:text-foreground flex items-center gap-1"
                onClick={() => setShowAlternatives((v) => !v)}
              >
                {showAlternatives ? (
                  <ChevronUp className="h-3 w-3" />
                ) : (
                  <ChevronDown className="h-3 w-3" />
                )}
                {showAlternatives
                  ? t("settings.localLlm.hideAlternatives")
                  : t("settings.localLlm.showAlternatives")}
              </button>
            )}
            {showAlternatives && (
              <div className="mt-2 space-y-1 border-t border-border/60 pt-2">
                {recommendation.alternatives.map((c) => {
                  const isChosen = recommended?.id === c.id
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => setChosen(c)}
                      className={`w-full text-left rounded-md px-2 py-1.5 text-[11px] transition-colors flex items-center justify-between gap-2 ${
                        isChosen
                          ? "bg-primary/10 text-foreground"
                          : "text-muted-foreground hover:bg-secondary"
                      }`}
                    >
                      <span className="truncate">{c.displayName}</span>
                      <span className="font-mono text-[10px] text-muted-foreground/80 shrink-0">
                        {formatBytesFromMb(c.sizeMb)}
                      </span>
                    </button>
                  )
                })}
              </div>
            )}
          </div>
        ) : (
          <div
            className={cn(
              "rounded-lg border border-dashed border-amber-500/30 bg-amber-500/10 text-[11px] text-amber-700 dark:text-amber-300",
              compact ? "p-2.5" : "p-3",
            )}
          >
            {t("settings.localLlm.hardware.insufficient")}
          </div>
        )}

        {!compact && <div className="flex items-center justify-end">{renderAction()}</div>}

        {error && <p className="text-[11px] text-destructive whitespace-pre-wrap">{error}</p>}
      </div>

      <InstallProgressDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        title={dialogTitle}
        subtitle={dialogSubtitle}
        frame={dialogFrame}
        logs={dialogLogs}
        done={dialogDone}
        error={dialogError}
        cancellable={false}
      />
    </>
  )
}
