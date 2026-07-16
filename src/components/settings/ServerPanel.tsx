import { useState, useEffect, useCallback, type ReactNode } from "react"
import { getTransport, useTransport } from "@/lib/transport-provider"
import { confirmTransportChange, switchToRemote, switchToEmbedded } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { logger } from "@/lib/logger"
import {
  patchFilesystemConfig,
  useFilesystemConfig,
  type FilesystemConfig,
} from "@/lib/filesystemConfig"
import {
  MonitorSmartphone,
  Globe,
  Check,
  Loader2,
  Wifi,
  CircleDot,
  RefreshCw,
  Network,
  Clock,
  Radio,
  MessageSquare,
  AlertTriangle,
} from "lucide-react"
import MetricCard from "@/components/common/MetricCard"
import {
  useServerStatus,
  formatServerUptime,
  formatActiveChatCounts,
  formatActiveConnectionsSub,
  totalActiveConnections,
} from "@/hooks/useServerStatus"

type ServerMode = "embedded" | "remote"

interface ServerConfig {
  serverMode: ServerMode
  remoteServerUrl: string
  remoteApiKey: string
  // Embedded server settings (from config.json)
  embeddedBindAddr: string
  embeddedApiKey: string
  embeddedKnowledgeAgentReadToken: string
}

const DEFAULT_EMBEDDED_ADDRESS = "127.0.0.1:8420"

const DEFAULT_CONFIG: ServerConfig = {
  serverMode: "embedded",
  remoteServerUrl: "",
  remoteApiKey: "",
  embeddedBindAddr: DEFAULT_EMBEDDED_ADDRESS,
  embeddedApiKey: "",
  embeddedKnowledgeAgentReadToken: "",
}

interface LoadedServerSecrets {
  embeddedApiKey: string
  hasEmbeddedApiKey: boolean
  embeddedKnowledgeAgentReadToken: string
  hasEmbeddedKnowledgeAgentReadToken: boolean
}

const EMPTY_LOADED_SECRETS: LoadedServerSecrets = {
  embeddedApiKey: "",
  hasEmbeddedApiKey: false,
  embeddedKnowledgeAgentReadToken: "",
  hasEmbeddedKnowledgeAgentReadToken: false,
}

/** Generate a random 32-char hex API key. */
function generateApiKey(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("")
}

function serializeOptionalSecret(
  current: string,
  loadedMaskedValue: string,
  hadLoadedValue: boolean,
): string | null {
  if (hadLoadedValue && current === loadedMaskedValue) return null
  const trimmed = current.trim()
  return trimmed ? trimmed : ""
}

export default function ServerPanel() {
  const { t } = useTranslation()

  const [config, setConfig] = useState<ServerConfig>(DEFAULT_CONFIG)
  const [loadedSecrets, setLoadedSecrets] = useState<LoadedServerSecrets>(EMPTY_LOADED_SECRETS)
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null)
  const [connected, setConnected] = useState<boolean | null>(null)

  const dirty = JSON.stringify(config) !== savedSnapshot

  // Load config on mount
  useEffect(() => {
    let cancelled = false

    Promise.all([
      getTransport().call<Record<string, unknown>>("get_user_config"),
      getTransport().call<Record<string, unknown>>("get_server_config"),
    ])
      .then(([userCfg, serverCfg]) => {
        if (cancelled) return
        const hasEmbeddedApiKey = Boolean(serverCfg.hasApiKey)
        const hasEmbeddedKnowledgeAgentReadToken = Boolean(serverCfg.hasKnowledgeAgentReadToken)
        const loaded: ServerConfig = {
          serverMode: (userCfg.serverMode as ServerMode) || "embedded",
          remoteServerUrl: (userCfg.remoteServerUrl as string) || "",
          remoteApiKey: (userCfg.remoteApiKey as string) || "",
          embeddedBindAddr: (serverCfg.bindAddr as string) || DEFAULT_EMBEDDED_ADDRESS,
          // Show masked key if exists, otherwise empty
          embeddedApiKey: hasEmbeddedApiKey ? (serverCfg.apiKey as string) || "" : "",
          embeddedKnowledgeAgentReadToken: hasEmbeddedKnowledgeAgentReadToken
            ? (serverCfg.knowledgeAgentReadToken as string) || ""
            : "",
        }
        setLoadedSecrets({
          embeddedApiKey: loaded.embeddedApiKey,
          hasEmbeddedApiKey,
          embeddedKnowledgeAgentReadToken: loaded.embeddedKnowledgeAgentReadToken,
          hasEmbeddedKnowledgeAgentReadToken,
        })
        setConfig(loaded)
        setSavedSnapshot(JSON.stringify(loaded))
      })
      .catch((e) => {
        logger.error("settings", "ServerPanel::load", "Failed to load config", e)
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Check connection status on mount and when config changes (debounced for URL typing)
  useEffect(() => {
    const timer = setTimeout(() => checkConnection(), 800)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config.serverMode, config.remoteServerUrl])

  const checkConnection = useCallback(async () => {
    try {
      const url =
        config.serverMode === "remote" && config.remoteServerUrl
          ? config.remoteServerUrl.replace(/\/+$/, "")
          : `http://${DEFAULT_EMBEDDED_ADDRESS}`
      const headers: Record<string, string> = {}
      if (config.serverMode === "remote" && config.remoteApiKey) {
        headers["Authorization"] = `Bearer ${config.remoteApiKey}`
      }
      const resp = await fetch(`${url}/api/health`, {
        method: "GET",
        headers,
        signal: AbortSignal.timeout(5000),
      })
      setConnected(resp.ok)
    } catch {
      setConnected(false)
    }
  }, [config.serverMode, config.remoteServerUrl, config.remoteApiKey])

  const handleSave = useCallback(async () => {
    // Confirm before persisting the requested mode. Otherwise cancelling the
    // editor guard would leave config pointing at a transport we never opened.
    if (!confirmTransportChange()) return
    setSaving(true)
    try {
      const embeddedApiKey = serializeOptionalSecret(
        config.embeddedApiKey,
        loadedSecrets.embeddedApiKey,
        loadedSecrets.hasEmbeddedApiKey,
      )
      const embeddedKnowledgeAgentReadToken = serializeOptionalSecret(
        config.embeddedKnowledgeAgentReadToken,
        loadedSecrets.embeddedKnowledgeAgentReadToken,
        loadedSecrets.hasEmbeddedKnowledgeAgentReadToken,
      )

      // Save user config (server mode, remote URL/key)
      const full = await getTransport().call<Record<string, unknown>>("get_user_config")
      await getTransport().call("save_user_config", {
        config: {
          ...full,
          serverMode: config.serverMode,
          remoteServerUrl: config.remoteServerUrl || null,
          remoteApiKey: config.remoteApiKey || null,
        },
      })

      // Save embedded server config (bind addr, api key) to config.json
      await getTransport().call("save_server_config", {
        config: {
          bindAddr: config.embeddedBindAddr || DEFAULT_EMBEDDED_ADDRESS,
          apiKey: embeddedApiKey,
          knowledgeAgentReadToken: embeddedKnowledgeAgentReadToken,
        },
      })

      // Switch transport based on mode
      if (config.serverMode === "remote" && config.remoteServerUrl) {
        switchToRemote(config.remoteServerUrl.replace(/\/+$/, ""), config.remoteApiKey || null, {
          dirtyConfirmed: true,
        })
      } else {
        switchToEmbedded({ dirtyConfirmed: true })
      }

      setLoadedSecrets((prev) => ({
        embeddedApiKey: embeddedApiKey === null ? prev.embeddedApiKey : embeddedApiKey,
        hasEmbeddedApiKey:
          embeddedApiKey === null ? prev.hasEmbeddedApiKey : embeddedApiKey.length > 0,
        embeddedKnowledgeAgentReadToken:
          embeddedKnowledgeAgentReadToken === null
            ? prev.embeddedKnowledgeAgentReadToken
            : embeddedKnowledgeAgentReadToken,
        hasEmbeddedKnowledgeAgentReadToken:
          embeddedKnowledgeAgentReadToken === null
            ? prev.hasEmbeddedKnowledgeAgentReadToken
            : embeddedKnowledgeAgentReadToken.length > 0,
      }))
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ServerPanel::save", "Failed to save server config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [config, loadedSecrets])

  const handleTestConnection = useCallback(async () => {
    setTesting(true)
    setTestResult(null)
    try {
      const url =
        config.serverMode === "remote" && config.remoteServerUrl
          ? config.remoteServerUrl.replace(/\/+$/, "")
          : `http://${DEFAULT_EMBEDDED_ADDRESS}`
      const headers: Record<string, string> = {}
      if (config.serverMode === "remote" && config.remoteApiKey) {
        headers["Authorization"] = `Bearer ${config.remoteApiKey}`
      }
      const resp = await fetch(`${url}/api/health`, {
        method: "GET",
        headers,
        signal: AbortSignal.timeout(10000),
      })
      if (resp.ok) {
        setTestResult({ ok: true, msg: `${resp.status} OK` })
        setConnected(true)
      } else {
        const text = await resp.text().catch(() => "")
        setTestResult({ ok: false, msg: `${resp.status} ${text}` })
        setConnected(false)
      }
    } catch (e) {
      setTestResult({ ok: false, msg: String(e) })
      setConnected(false)
    } finally {
      setTesting(false)
    }
  }, [config])

  const modeOptions: {
    value: ServerMode
    icon: React.ReactNode
    label: string
    desc: string
  }[] = [
    {
      value: "embedded",
      icon: <MonitorSmartphone className="h-4 w-4" />,
      label: t("settings.serverModeEmbedded"),
      desc: t("settings.serverModeEmbeddedDesc"),
    },
    {
      value: "remote",
      icon: <Globe className="h-4 w-4" />,
      label: t("settings.serverModeRemote"),
      desc: t("settings.serverModeRemoteDesc"),
    },
  ]

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="w-full space-y-6">
        {/* Header */}
        <div>
          <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.server")}</h2>
          <p className="text-xs text-muted-foreground">{t("settings.serverDesc")}</p>
        </div>

        {/* Connection Status */}
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">
            {t("settings.serverConnectionStatus")}:
          </span>
          {connected === null ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
          ) : connected ? (
            <span className="flex items-center gap-1.5 text-sm text-green-600">
              <CircleDot className="h-3.5 w-3.5" />
              {t("settings.serverConnected")}
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-sm text-destructive">
              <CircleDot className="h-3.5 w-3.5" />
              {t("settings.serverDisconnected")}
            </span>
          )}
        </div>

        {/* Runtime Status — live snapshot of the embedded server. Shown in
            both embedded and remote modes; the underlying command is routed
            by the Transport layer so remote servers answer via
            GET /api/server/status (unauthenticated). */}
        <RuntimeStatusSection />

        {/* File-browser remote-write gate */}
        <FilesystemSection />

        {/* Mode Selector */}
        <div className="space-y-3">
          <div>
            <h3 className="text-sm font-medium">{t("settings.serverMode")}</h3>
            <p className="text-xs text-muted-foreground mt-0.5">{t("settings.serverModeDesc")}</p>
          </div>
          <div className="space-y-1.5">
            {modeOptions.map((opt) => (
              <div
                key={opt.value}
                className={cn(
                  "flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-colors",
                  config.serverMode === opt.value
                    ? "bg-primary/10 border border-primary/30"
                    : "hover:bg-secondary/40 border border-transparent",
                )}
                onClick={() => setConfig((prev) => ({ ...prev, serverMode: opt.value }))}
              >
                <div
                  className={cn(
                    "shrink-0",
                    config.serverMode === opt.value ? "text-primary" : "text-muted-foreground",
                  )}
                >
                  {opt.icon}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium">{opt.label}</div>
                  <div className="text-xs text-muted-foreground">{opt.desc}</div>
                </div>
                <div
                  className={cn(
                    "h-4 w-4 rounded-full border-2 shrink-0 transition-colors",
                    config.serverMode === opt.value
                      ? "border-primary bg-primary"
                      : "border-muted-foreground/30",
                  )}
                >
                  {config.serverMode === opt.value && (
                    <div className="h-full w-full flex items-center justify-center">
                      <div className="h-1.5 w-1.5 rounded-full bg-primary-foreground" />
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Embedded mode: configurable bind address + API key */}
        {config.serverMode === "embedded" && (
          <div className="space-y-3">
            <div className="space-y-1.5">
              <span className="text-xs text-muted-foreground">
                {t("settings.serverEmbeddedBind")}
              </span>
              <Input
                value={config.embeddedBindAddr}
                placeholder={DEFAULT_EMBEDDED_ADDRESS}
                onChange={(e) =>
                  setConfig((prev) => ({
                    ...prev,
                    embeddedBindAddr: e.target.value,
                  }))
                }
              />
              <p className="text-xs text-muted-foreground">
                {t("settings.serverEmbeddedBindDesc")}
              </p>
            </div>
            <div className="space-y-1.5">
              <span className="text-xs text-muted-foreground">
                {t("settings.serverEmbeddedApiKey")}
              </span>
              <div className="flex gap-2">
                <Input
                  type="password"
                  className="flex-1"
                  value={config.embeddedApiKey}
                  placeholder={t("settings.serverEmbeddedApiKeyPlaceholder")}
                  onChange={(e) =>
                    setConfig((prev) => ({
                      ...prev,
                      embeddedApiKey: e.target.value,
                    }))
                  }
                />
                <Button
                  variant="outline"
                  size="sm"
                  className="shrink-0"
                  onClick={() =>
                    setConfig((prev) => ({
                      ...prev,
                      embeddedApiKey: generateApiKey(),
                    }))
                  }
                >
                  <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
                  {t("settings.serverGenerateApiKey")}
                </Button>
              </div>
            </div>
            <div className="space-y-1.5">
              <span className="text-xs text-muted-foreground">
                {t("settings.serverKnowledgeAgentReadToken")}
              </span>
              <div className="flex gap-2">
                <Input
                  type="password"
                  className="flex-1"
                  value={config.embeddedKnowledgeAgentReadToken}
                  placeholder={t("settings.serverKnowledgeAgentReadTokenPlaceholder")}
                  onChange={(e) =>
                    setConfig((prev) => ({
                      ...prev,
                      embeddedKnowledgeAgentReadToken: e.target.value,
                    }))
                  }
                />
                <Button
                  variant="outline"
                  size="sm"
                  className="shrink-0"
                  onClick={() =>
                    setConfig((prev) => ({
                      ...prev,
                      embeddedKnowledgeAgentReadToken: generateApiKey(),
                    }))
                  }
                >
                  <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
                  {t("settings.serverGenerateApiKey")}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {t("settings.serverKnowledgeAgentReadTokenDesc")}
              </p>
            </div>
            <p className="text-xs text-amber-600 dark:text-amber-400">
              {t("settings.serverRestartRequired")}
            </p>
          </div>
        )}

        {/* Remote mode: URL + API key inputs */}
        {config.serverMode === "remote" && (
          <div className="space-y-3">
            <div className="space-y-1.5">
              <span className="text-xs text-muted-foreground">{t("settings.serverRemoteUrl")}</span>
              <Input
                value={config.remoteServerUrl}
                placeholder={t("settings.serverRemoteUrlPlaceholder")}
                onChange={(e) =>
                  setConfig((prev) => ({
                    ...prev,
                    remoteServerUrl: e.target.value,
                  }))
                }
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-xs text-muted-foreground">{t("settings.serverApiKey")}</span>
              <Input
                type="password"
                value={config.remoteApiKey}
                placeholder={t("settings.serverApiKeyPlaceholder")}
                onChange={(e) =>
                  setConfig((prev) => ({
                    ...prev,
                    remoteApiKey: e.target.value,
                  }))
                }
              />
            </div>
          </div>
        )}

        {/* Save + Test buttons */}
        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            onClick={handleSave}
            disabled={(!dirty && saveStatus === "idle") || saving}
            className={cn(
              saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
              saveStatus === "failed" &&
                "bg-destructive/10 text-destructive hover:bg-destructive/20",
            )}
          >
            {saving ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.saving")}
              </span>
            ) : saveStatus === "saved" ? (
              <span className="flex items-center gap-1.5">
                <Check className="h-3.5 w-3.5" />
                {t("common.saved")}
              </span>
            ) : saveStatus === "failed" ? (
              t("common.saveFailed")
            ) : (
              t("common.save")
            )}
          </Button>

          <Button
            variant="secondary"
            size="sm"
            disabled={
              testing || (config.serverMode === "remote" && !config.remoteServerUrl?.trim())
            }
            onClick={handleTestConnection}
          >
            {testing ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.testing")}
              </span>
            ) : (
              <span className="flex items-center gap-1.5">
                <Wifi className="h-3.5 w-3.5" />
                {t("settings.serverTestConnection")}
              </span>
            )}
          </Button>
        </div>

        {/* Test result */}
        {testResult && (
          <div
            className={cn(
              "px-3 py-2 rounded-md text-xs",
              testResult.ok
                ? "bg-green-500/10 text-green-600"
                : "bg-destructive/10 text-destructive",
            )}
          >
            <div className="font-medium">
              {testResult.ok ? t("settings.serverTestSuccess") : t("settings.serverTestFailed")}
            </div>
            <pre className="mt-1 whitespace-pre-wrap break-all opacity-80">{testResult.msg}</pre>
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Shared filesystem settings. Local desktop reads the local config; Web and
 * remote desktop read and update the connected Server config.
 */
function FilesystemSection() {
  const { t } = useTranslation()
  const transport = useTransport()
  const { config } = useFilesystemConfig()
  const [status, setStatus] = useState<"idle" | "saved" | "failed">("idle")

  const save = useCallback(
    async (next: FilesystemConfig) => {
      try {
        await patchFilesystemConfig(transport, {
          allowRemoteWrites: next.allowRemoteWrites,
        })
        setStatus("saved")
        setTimeout(() => setStatus("idle"), 2000)
      } catch (e) {
        logger.error("settings", "ServerPanel::fs", "save failed", e)
        setStatus("failed")
        setTimeout(() => setStatus("idle"), 2000)
      }
    },
    [transport],
  )

  return (
    <section className="space-y-2">
      <div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2.5">
        <Switch
          checked={config.allowRemoteWrites}
          onCheckedChange={(allowRemoteWrites) => void save({ ...config, allowRemoteWrites })}
          className="mt-0.5"
        />
        <div className="flex-1 space-y-0.5">
          <h3 className="text-sm font-medium">
            {t("settings.fsRemoteWrites", "Allow remote file writes")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.fsRemoteWritesDesc",
              "Let HTTP clients create, edit, delete, and upload files in the project working directory. Off by default — the desktop app can always write locally regardless of this setting.",
            )}
          </p>
        </div>
      </div>
      {status === "saved" ? (
        <span className="flex items-center gap-1 text-xs text-green-600">
          <Check className="h-3 w-3" />
          {t("common.saved", "Saved")}
        </span>
      ) : status === "failed" ? (
        <span className="text-xs text-destructive">{t("common.saveFailed", "Save failed")}</span>
      ) : null}
    </section>
  )
}

function RuntimeStatusSection() {
  const { t } = useTranslation()
  const { status, loading, error } = useServerStatus(3000)

  let body: ReactNode
  if (loading && !status && !error) {
    body = (
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="h-[58px] rounded-lg bg-muted/50 animate-pulse" />
        ))}
      </div>
    )
  } else if (!status) {
    body = (
      <div className="flex items-center gap-2 text-xs text-muted-foreground px-3 py-2 rounded-md bg-muted/40">
        <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{error ?? t("settings.serverNotStarted")}</span>
      </div>
    )
  } else if (status.startupError) {
    body = (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2.5 space-y-1.5">
        <div className="flex items-center gap-2 text-sm font-medium text-destructive">
          <AlertTriangle className="h-4 w-4 shrink-0" />
          {t("settings.serverStartupError")}
        </div>
        <pre className="text-xs text-destructive/90 whitespace-pre-wrap break-all">
          {status.startupError}
        </pre>
        <p className="text-[11px] text-destructive/80">{t("settings.serverRestartRequired")}</p>
      </div>
    )
  } else {
    const wsTotal = totalActiveConnections(status)
    const wsSub = formatActiveConnectionsSub(status, t)
    body = (
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <MetricCard
          icon={Network}
          label={t("settings.serverBoundAddr")}
          value={status.boundAddr ?? t("settings.serverNotStarted")}
          colorClass="text-indigo-500"
          bgClass="bg-indigo-500/10"
        />
        <MetricCard
          icon={Clock}
          label={t("settings.serverUptime")}
          value={formatServerUptime(status.uptimeSecs)}
          colorClass="text-green-500"
          bgClass="bg-green-500/10"
        />
        <MetricCard
          icon={Radio}
          label={t("settings.serverActiveWebSockets")}
          value={String(wsTotal)}
          subValue={wsSub}
          colorClass="text-amber-500"
          bgClass="bg-amber-500/10"
        />
        <MetricCard
          icon={MessageSquare}
          label={t("settings.serverActiveChatStreams")}
          value={String(status.activeChatCounts.total)}
          subValue={formatActiveChatCounts(status.activeChatCounts, t) ?? undefined}
          colorClass="text-purple-500"
          bgClass="bg-purple-500/10"
        />
      </div>
    )
  }

  return (
    <section className="space-y-2">
      <h3 className="text-sm font-medium">{t("settings.serverRuntimeStatus")}</h3>
      {body}
    </section>
  )
}
