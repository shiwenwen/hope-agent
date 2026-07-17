import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Globe, Laptop, Loader2, Wifi } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { confirmTransportChange, getTransport, switchToRemote } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface RemoteConnectPanelProps {
  remoteUrl: string
  remoteApiKey: string
  onChange: (patch: { remoteUrl?: string; remoteApiKey?: string }) => void
  /** Called after the remote connect succeeds — the wizard finishes. */
  onRemoteConnected: () => void
}

/** Shared remote connection form used by the welcome page's secondary action. */
export function RemoteConnectPanel({
  remoteUrl,
  remoteApiKey,
  onChange,
  onRemoteConnected,
}: RemoteConnectPanelProps) {
  const { t } = useTranslation()
  const [phase, setPhase] = useState<"idle" | "testing" | "connecting">("idle")
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const busy = phase !== "idle"
  const trimmedUrl = remoteUrl.trim().replace(/\/+$/, "")

  async function probe(): Promise<{ ok: boolean; msg: string }> {
    const headers: Record<string, string> = {}
    const key = remoteApiKey.trim()
    if (key) headers["Authorization"] = `Bearer ${key}`
    const resp = await fetch(`${trimmedUrl}/api/health`, {
      method: "GET",
      headers,
      signal: AbortSignal.timeout(10000),
    })
    if (resp.ok) return { ok: true, msg: `${resp.status} OK` }
    const text = await resp.text().catch(() => "")
    return { ok: false, msg: `${resp.status} ${text}` }
  }

  async function handleTest() {
    if (!remoteUrl.trim()) return
    setPhase("testing")
    setResult(null)
    try {
      setResult(await probe())
    } catch (e) {
      setResult({ ok: false, msg: String(e) })
    } finally {
      setPhase("idle")
    }
  }

  async function handleConnect() {
    if (!remoteUrl.trim()) return
    setPhase("connecting")
    setResult(null)
    try {
      const probed = await probe()
      if (!probed.ok) {
        setResult(probed)
        return
      }
      if (!confirmTransportChange()) return
      const finalKey = remoteApiKey.trim() || null
      const full = await getTransport().call<Record<string, unknown>>("get_user_config")
      await getTransport().call("save_user_config", {
        config: {
          ...full,
          serverMode: "remote",
          remoteServerUrl: trimmedUrl,
          remoteApiKey: finalKey,
        },
      })
      switchToRemote(trimmedUrl, finalKey, { dirtyConfirmed: true })
      onRemoteConnected()
    } catch (e) {
      logger.error("onboarding", "RemoteConnectPanel::connect", "remote connect failed", e)
      setResult({ ok: false, msg: String(e) })
    } finally {
      setPhase("idle")
    }
  }

  return (
    <div className="space-y-3 rounded-xl border border-border bg-muted/30 p-5">
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-muted-foreground">
          {t("provider.remoteServerUrl")}
        </label>
        <Input
          value={remoteUrl}
          onChange={(e) => onChange({ remoteUrl: e.target.value })}
          placeholder="http://192.168.1.10:8420"
        />
      </div>
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-muted-foreground">
          {t("provider.remoteApiKey")}{" "}
          <span className="text-muted-foreground/70">({t("provider.optional")})</span>
        </label>
        <Input
          type="password"
          value={remoteApiKey}
          onChange={(e) => onChange({ remoteApiKey: e.target.value })}
          placeholder={t("provider.remoteApiKeyPlaceholder")}
        />
      </div>
      {result && (
        <div
          className={
            result.ok
              ? "rounded-md bg-green-500/10 px-3 py-2 text-xs text-green-600"
              : "rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive"
          }
        >
          <div className="font-medium">
            {result.ok ? t("provider.remoteTestSuccess") : t("provider.remoteTestFailed")}
          </div>
          <pre className="mt-1 whitespace-pre-wrap break-all opacity-80">{result.msg}</pre>
        </div>
      )}
      <div className="flex items-center gap-2 pt-1">
        <Button
          variant="secondary"
          size="sm"
          onClick={handleTest}
          disabled={!remoteUrl.trim() || busy}
        >
          {phase === "testing" ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.testing")}
            </span>
          ) : (
            <span className="flex items-center gap-1.5">
              <Wifi className="h-3.5 w-3.5" />
              {t("provider.testConnection")}
            </span>
          )}
        </Button>
        <Button size="sm" onClick={handleConnect} disabled={!remoteUrl.trim() || busy}>
          {phase === "connecting" ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("provider.remoteConnecting")}
            </span>
          ) : (
            t("provider.remoteConnect")
          )}
        </Button>
      </div>
    </div>
  )
}

interface ModeStepProps extends RemoteConnectPanelProps {
  mode: "local" | "remote" | undefined
  onChange: (patch: {
    mode?: "local" | "remote"
    remoteUrl?: string
    remoteApiKey?: string
  }) => void
}

/** Legacy full mode chooser retained for reusable remote-connection behavior. */
export function ModeStep({
  mode,
  remoteUrl,
  remoteApiKey,
  onChange,
  onRemoteConnected,
}: ModeStepProps) {
  const { t } = useTranslation()

  return (
    <div className="space-y-6 px-8 py-8">
      <div className="space-y-1 text-center">
        <h2 className="text-2xl font-semibold tracking-tight">{t("onboarding.mode.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.mode.subtitle")}</p>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <button
          type="button"
          onClick={() => onChange({ mode: "local" })}
          className={`rounded-xl border p-5 text-left transition-colors ${
            mode === "local"
              ? "border-border bg-secondary/70"
              : "border-border hover:bg-secondary/40"
          }`}
        >
          <div className="mb-2 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Laptop className="h-5 w-5" />
            </div>
            <div className="font-semibold">{t("onboarding.mode.localTitle")}</div>
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("onboarding.mode.localDesc")}
          </p>
        </button>

        <button
          type="button"
          onClick={() => onChange({ mode: "remote" })}
          className={`rounded-xl border p-5 text-left transition-colors ${
            mode === "remote"
              ? "border-border bg-secondary/70"
              : "border-border hover:bg-secondary/40"
          }`}
        >
          <div className="mb-2 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Globe className="h-5 w-5" />
            </div>
            <div className="font-semibold">{t("onboarding.mode.remoteTitle")}</div>
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("onboarding.mode.remoteDesc")}
          </p>
        </button>
      </div>

      {mode === "remote" && (
        <RemoteConnectPanel
          remoteUrl={remoteUrl}
          remoteApiKey={remoteApiKey}
          onChange={onChange}
          onRemoteConnected={onRemoteConnected}
        />
      )}
    </div>
  )
}
