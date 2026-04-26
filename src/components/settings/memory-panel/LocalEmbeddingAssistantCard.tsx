import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Cpu,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  Sparkles,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { parsePayload } from "@/lib/transport"
import { withEventListener } from "@/lib/transport-events"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { formatBytesFromMb } from "@/lib/format"
import { cn } from "@/lib/utils"
import {
  InstallProgressDialog,
  type ProgressFrame,
} from "@/components/settings/local-llm/InstallProgressDialog"
import type { EmbeddingConfig, OllamaEmbeddingModel } from "./types"

type OllamaPhase = "not-installed" | "installed" | "running"
type InstallProgressKind = "step" | "log" | "error"

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

const EVENT_LOCAL_LLM_INSTALL_PROGRESS = "local_llm:install_progress"
const EVENT_LOCAL_EMBEDDING_PULL_PROGRESS = "local_embedding:pull_progress"
const MAX_DIALOG_LOG_LINES = 240

const PHASE_KEY: Record<string, string> = {
  starting: "settings.localLlm.phases.starting",
  "pulling manifest": "settings.localLlm.phases.pullingManifest",
  downloading: "settings.localLlm.phases.downloading",
  "verifying digest": "settings.localLlm.phases.verifying",
  "writing manifest": "settings.localLlm.phases.writingManifest",
  success: "settings.localLlm.phases.success",
  "configure-embedding": "settings.localEmbedding.phases.configureEmbedding",
  done: "settings.localLlm.phases.done",
}

function formatLogLine(message: string): string {
  return `[${new Date().toLocaleTimeString()}] ${message}`
}

export default function LocalEmbeddingAssistantCard({
  onActivated,
}: {
  onActivated: (config: EmbeddingConfig) => void
}) {
  const { t } = useTranslation()
  const [models, setModels] = useState<OllamaEmbeddingModel[]>([])
  const [ollama, setOllama] = useState<OllamaStatus | null>(null)
  const [chosen, setChosen] = useState<OllamaEmbeddingModel | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [showAlternatives, setShowAlternatives] = useState(false)
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
      const [nextModels, status] = await Promise.all([
        getTransport().call<OllamaEmbeddingModel[]>("local_embedding_list_models"),
        getTransport().call<OllamaStatus>("local_llm_detect_ollama"),
      ])
      setModels(nextModels)
      setOllama(status)
      setChosen((current) =>
        current ? (nextModels.find((model) => model.id === current.id) ?? current) : current,
      )
    } catch (e) {
      logger.error("settings", "LocalEmbeddingAssistant::refresh", "Failed to refresh", e)
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

  const activateModel = useCallback(
    async (model: OllamaEmbeddingModel) => {
      if (ollama?.phase === "not-installed" && !ollama.installScriptSupported) {
        openDownloadPage()
        return
      }

      setBusy(true)
      setError(null)
      setDialogOpen(true)
      setDialogTitle(t("settings.localEmbedding.install.title"))
      setDialogSubtitle(model.id)
      setDialogFrame({
        phase: "starting",
        message: t("settings.localLlm.phases.starting"),
        percent: null,
      })
      setDialogLogs([])
      setDialogDone(false)
      setDialogError(null)

      const handleInstallProgress = (raw: unknown) => {
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

      const handlePullProgress = (raw: unknown) => {
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
        if (ollama?.phase === "not-installed") {
          await withEventListener(EVENT_LOCAL_LLM_INSTALL_PROGRESS, handleInstallProgress, () =>
            getTransport().call("local_llm_install_ollama"),
          )
        }

        if (ollama?.phase !== "running") {
          const label = t("settings.localLlm.buttons.startOllama")
          setDialogFrame({ phase: "starting", message: label, percent: null })
          appendDialogLog(label)
          await getTransport().call("local_llm_start_ollama")
        }

        const config = await withEventListener(
          EVENT_LOCAL_EMBEDDING_PULL_PROGRESS,
          handlePullProgress,
          () =>
            getTransport().call<EmbeddingConfig>("local_embedding_pull_and_activate", {
              model,
            }),
        )

        onActivated(config)
        setDialogDone(true)
        appendDialogLog(t("settings.localLlm.phases.done"))
        await refresh()
        setTimeout(() => setDialogOpen(false), 800)
      } catch (e) {
        const msg = String(e)
        setDialogError(msg)
        appendDialogLog(msg)
        setError(t("settings.localEmbedding.error.activateFailed", { message: msg }))
      } finally {
        setBusy(false)
      }
    },
    [appendDialogLog, ollama, onActivated, openDownloadPage, phaseLabel, refresh, t],
  )

  const recommended = chosen ?? models.find((model) => model.recommended) ?? models[0] ?? null

  if (!recommended) {
    return (
      <div className="rounded-lg border border-dashed border-border bg-card/40 p-3">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.localEmbedding.detecting")}
        </div>
      </div>
    )
  }

  const primaryAction = () => {
    if (!ollama) return null

    if (ollama?.phase === "not-installed" && !ollama.installScriptSupported) {
      return (
        <Button variant="secondary" size="sm" onClick={openDownloadPage}>
          <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
          {t("settings.localEmbedding.buttons.downloadOllama")}
        </Button>
      )
    }

    const label =
      recommended.installed && ollama?.phase === "running"
        ? t("settings.localEmbedding.buttons.enable", { model: recommended.displayName })
        : t("settings.localEmbedding.buttons.activate", { model: recommended.displayName })

    return (
      <Button size="sm" onClick={() => void activateModel(recommended)} disabled={busy}>
        {busy ? (
          <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
        ) : (
          <Download className="h-3.5 w-3.5 mr-1.5" />
        )}
        {label}
      </Button>
    )
  }

  return (
    <>
      <div className="rounded-lg border border-primary/25 bg-primary/5 p-3 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-3 min-w-0">
            <div className="w-8 h-8 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0">
              <Sparkles className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-foreground">
                {t("settings.localEmbedding.title")}
              </div>
              <div className="text-[11px] text-muted-foreground mt-0.5">
                {t("settings.localEmbedding.subtitle")}
              </div>
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

        <div className="rounded-lg border border-border/60 bg-card p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-medium text-foreground">
                  {recommended.displayName}
                </span>
                {recommended.recommended && (
                  <span className="text-[10px] uppercase tracking-wide text-emerald-700 dark:text-emerald-300 bg-emerald-500/10 border border-emerald-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localEmbedding.recommended")}
                  </span>
                )}
                {recommended.installed && (
                  <span className="text-[10px] uppercase tracking-wide text-sky-700 dark:text-sky-300 bg-sky-500/10 border border-sky-500/25 px-1.5 py-0.5 rounded">
                    {t("settings.localEmbedding.installed")}
                  </span>
                )}
              </div>
              <div className="text-[11px] text-muted-foreground mt-1 flex items-center gap-1.5 flex-wrap">
                <Cpu className="h-3 w-3" />
                <span>{formatBytesFromMb(recommended.sizeMb)}</span>
                <span>·</span>
                <span>
                  {t("settings.localEmbedding.dimensions", { n: recommended.dimensions })}
                </span>
                <span>·</span>
                <span>
                  {t("settings.localEmbedding.contextWindow", {
                    n: recommended.contextWindow.toLocaleString(),
                  })}
                </span>
                <span>·</span>
                <span>{recommended.languages.join(", ")}</span>
                {recommended.minOllamaVersion && (
                  <>
                    <span>·</span>
                    <span>Ollama {recommended.minOllamaVersion}+</span>
                  </>
                )}
              </div>
            </div>
            <div className="shrink-0">{primaryAction()}</div>
          </div>

          {models.length > 1 && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-2 h-7 px-2 text-[11px] text-muted-foreground"
              onClick={() => setShowAlternatives((v) => !v)}
            >
              {showAlternatives ? (
                <ChevronUp className="h-3 w-3 mr-1" />
              ) : (
                <ChevronDown className="h-3 w-3 mr-1" />
              )}
              {showAlternatives
                ? t("settings.localEmbedding.hideAlternatives")
                : t("settings.localEmbedding.showAlternatives")}
            </Button>
          )}

          {showAlternatives && (
            <div className="mt-2 space-y-1 border-t border-border/60 pt-2">
              {models.map((model) => {
                const active = model.id === recommended.id
                return (
                  <Button
                    key={model.id}
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setChosen(model)}
                    className={cn(
                      "w-full h-auto justify-between px-2 py-1.5 text-left text-[11px]",
                      active
                        ? "bg-primary/10 text-foreground"
                        : "text-muted-foreground hover:bg-secondary",
                    )}
                  >
                    <span className="truncate">{model.displayName}</span>
                    <span className="font-mono text-[10px] text-muted-foreground/80 shrink-0">
                      {formatBytesFromMb(model.sizeMb)} · {model.dimensions}d
                    </span>
                  </Button>
                )
              })}
            </div>
          )}
        </div>

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
