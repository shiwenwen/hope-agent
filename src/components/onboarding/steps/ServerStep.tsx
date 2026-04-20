import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Copy, Laptop, Wifi } from "lucide-react"

import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"

import { getTransport } from "@/lib/transport-provider"

interface ServerStepProps {
  bindMode: "local" | "lan"
  apiKey: string
  apiKeyEnabled: boolean
  onChange: (patch: { bindMode: "local" | "lan"; apiKey: string; apiKeyEnabled: boolean }) => void
}

/**
 * Step 7 — bind address + optional API key.
 *
 * Radios expand only two user-friendly choices; the raw bind string
 * ("0.0.0.0:8420" vs "127.0.0.1:8420") is kept out of sight. LAN mode
 * flips the API-Key switch on by default since an exposed port without
 * auth is a sharp edge.
 */
export function ServerStep({ bindMode, apiKey, apiKeyEnabled, onChange }: ServerStepProps) {
  const { t } = useTranslation()
  const [localIps, setLocalIps] = useState<string[]>([])
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    void (async () => {
      try {
        const ips = await getTransport().call<string[]>("list_local_ips")
        if (Array.isArray(ips)) setLocalIps(ips)
      } catch {
        setLocalIps([])
      }
    })()
  }, [])

  function update(patch: Partial<{ bindMode: "local" | "lan"; apiKey: string; apiKeyEnabled: boolean }>) {
    onChange({
      bindMode: patch.bindMode ?? bindMode,
      apiKey: patch.apiKey ?? apiKey,
      apiKeyEnabled: patch.apiKeyEnabled ?? apiKeyEnabled,
    })
  }

  async function regenerateKey() {
    try {
      const k = await getTransport().call<string>("generate_api_key")
      update({ apiKey: k, apiKeyEnabled: true })
    } catch {
      /* user can type a key manually instead */
    }
  }

  /**
   * Toggle the API-key switch.
   *
   * When flipping ON for the first time (no key yet), auto-generate one
   * so the wizard never ends up in the "apiKeyEnabled=true but no key"
   * state — which previously persisted as "clear the key" in the apply
   * step, silently dropping the user's intent. Auto-generation lets the
   * user edit / regenerate the key afterwards before hitting Next.
   */
  async function toggleApiKey(enabled: boolean) {
    if (enabled && !apiKey) {
      await regenerateKey()
    } else {
      update({ apiKeyEnabled: enabled })
    }
  }

  const previewHost = bindMode === "lan" && localIps[0] ? localIps[0] : "localhost"
  const previewUrl = apiKeyEnabled && apiKey
    ? `http://${previewHost}:8420/?token=${apiKey}`
    : `http://${previewHost}:8420/`

  async function copyPreview() {
    try {
      await navigator.clipboard.writeText(previewUrl)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      /* ignore */
    }
  }

  return (
    <div className="px-6 py-6 space-y-5 max-w-xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.server.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.server.subtitle")}</p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <button
          type="button"
          onClick={() => update({ bindMode: "local" })}
          className={`rounded-lg border-2 px-4 py-3 text-left transition-all ${
            bindMode === "local"
              ? "border-primary bg-primary/5"
              : "border-border hover:border-foreground/30"
          }`}
        >
          <div className="flex items-center gap-2 mb-1">
            <Laptop className="h-4 w-4" />
            <span className="font-medium text-sm">{t("onboarding.server.local")}</span>
          </div>
          <p className="text-xs text-muted-foreground">{t("onboarding.server.localHint")}</p>
        </button>
        <button
          type="button"
          onClick={() => update({ bindMode: "lan", apiKeyEnabled: true })}
          className={`rounded-lg border-2 px-4 py-3 text-left transition-all ${
            bindMode === "lan"
              ? "border-primary bg-primary/5"
              : "border-border hover:border-foreground/30"
          }`}
        >
          <div className="flex items-center gap-2 mb-1">
            <Wifi className="h-4 w-4" />
            <span className="font-medium text-sm">{t("onboarding.server.lan")}</span>
          </div>
          <p className="text-xs text-muted-foreground">{t("onboarding.server.lanHint")}</p>
        </button>
      </div>

      <div className="rounded-md border border-border px-4 py-3 space-y-2">
        <div className="flex items-center justify-between">
          <Label htmlFor="onb-apikey-toggle" className="text-sm font-medium">
            {t("onboarding.server.apiKeyLabel")}
          </Label>
          <Switch
            id="onb-apikey-toggle"
            checked={apiKeyEnabled}
            onCheckedChange={(v) => void toggleApiKey(v)}
          />
        </div>
        {apiKeyEnabled && (
          <div className="flex gap-2">
            <Input
              value={apiKey}
              onChange={(e) => update({ apiKey: e.target.value })}
              placeholder="hope_..."
              className="font-mono text-xs"
            />
            <Button variant="outline" size="sm" onClick={regenerateKey}>
              {t("onboarding.server.generate")}
            </Button>
          </div>
        )}
        <p className="text-xs text-muted-foreground">{t("onboarding.server.apiKeyHint")}</p>
      </div>

      <div className="rounded-md border border-border bg-muted/40 px-4 py-3 space-y-1">
        <div className="flex items-center justify-between">
          <Label className="text-xs text-muted-foreground">{t("onboarding.server.previewLabel")}</Label>
          <Button variant="ghost" size="sm" onClick={copyPreview}>
            <Copy className="h-3.5 w-3.5 mr-1" />
            {copied ? t("onboarding.server.copied") : t("onboarding.server.copy")}
          </Button>
        </div>
        <code className="text-xs break-all">{previewUrl}</code>
      </div>

      <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-xs text-amber-700 dark:text-amber-300">
        {t("onboarding.server.restartHint")}
      </div>
    </div>
  )
}
