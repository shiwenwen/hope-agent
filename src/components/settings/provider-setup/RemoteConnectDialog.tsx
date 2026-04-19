import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { getTransport, switchToRemote } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Globe, Loader2, Wifi } from "lucide-react"

type Phase = "idle" | "testing" | "connecting"

interface RemoteConnectDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConnected: () => void
}

export function RemoteConnectDialog({
  open,
  onOpenChange,
  onConnected,
}: RemoteConnectDialogProps) {
  const { t } = useTranslation()
  const [url, setUrl] = useState("")
  const [apiKey, setApiKey] = useState("")
  const [phase, setPhase] = useState<Phase>("idle")
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const busy = phase !== "idle"
  const normalizedUrl = () => url.trim().replace(/\/+$/, "")

  async function probe(): Promise<{ ok: boolean; msg: string }> {
    const headers: Record<string, string> = {}
    const key = apiKey.trim()
    if (key) headers["Authorization"] = `Bearer ${key}`
    const resp = await fetch(`${normalizedUrl()}/api/health`, {
      method: "GET",
      headers,
      signal: AbortSignal.timeout(10000),
    })
    if (resp.ok) return { ok: true, msg: `${resp.status} OK` }
    const text = await resp.text().catch(() => "")
    return { ok: false, msg: `${resp.status} ${text}` }
  }

  async function handleTest() {
    if (!url.trim()) return
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
    if (!url.trim()) return
    setPhase("connecting")
    setResult(null)
    try {
      const probed = await probe()
      if (!probed.ok) {
        setResult(probed)
        return
      }
      const finalUrl = normalizedUrl()
      const finalKey = apiKey.trim() || null
      const full = await getTransport().call<Record<string, unknown>>("get_user_config")
      await getTransport().call("save_user_config", {
        config: {
          ...full,
          serverMode: "remote",
          remoteServerUrl: finalUrl,
          remoteApiKey: finalKey,
        },
      })
      switchToRemote(finalUrl, finalKey)
      onOpenChange(false)
      onConnected()
    } catch (e) {
      logger.error("app", "RemoteConnectDialog::connect", "Failed to connect remote", e)
      setResult({ ok: false, msg: String(e) })
    } finally {
      setPhase("idle")
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setResult(null)
        onOpenChange(next)
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Globe className="h-4 w-4" />
            {t("provider.connectRemoteServer")}
          </DialogTitle>
          <DialogDescription>
            {t("provider.connectRemoteServerDesc")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="space-y-1.5">
            <label className="text-xs text-muted-foreground">
              {t("provider.remoteServerUrl")}
            </label>
            <Input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="http://192.168.1.10:8420"
              autoFocus
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-xs text-muted-foreground">
              {t("provider.remoteApiKey")}{" "}
              <span className="text-muted-foreground/70">
                ({t("provider.optional")})
              </span>
            </label>
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={t("provider.remoteApiKeyPlaceholder")}
            />
          </div>
          {result && (
            <div
              className={
                result.ok
                  ? "px-3 py-2 rounded-md text-xs bg-green-500/10 text-green-600"
                  : "px-3 py-2 rounded-md text-xs bg-destructive/10 text-destructive"
              }
            >
              <div className="font-medium">
                {result.ok
                  ? t("provider.remoteTestSuccess")
                  : t("provider.remoteTestFailed")}
              </div>
              <pre className="mt-1 whitespace-pre-wrap break-all opacity-80">
                {result.msg}
              </pre>
            </div>
          )}
        </div>

        <DialogFooter className="gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={handleTest}
            disabled={!url.trim() || busy}
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
          <Button size="sm" onClick={handleConnect} disabled={!url.trim() || busy}>
            {phase === "connecting" ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("provider.remoteConnecting")}
              </span>
            ) : (
              t("provider.remoteConnect")
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
