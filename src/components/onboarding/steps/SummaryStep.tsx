import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Copy, ExternalLink, Minus, Sparkles } from "lucide-react"

import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"

import type { OnboardingDraft, OnboardingStepKey } from "../types"

interface SummaryStepProps {
  draft: OnboardingDraft
  skipped: Set<OnboardingStepKey>
}

/**
 * Step 9 — final recap + Web GUI deep link.
 *
 * Shows a per-step status (done vs skipped) so the user confirms what
 * got persisted. Also queries `list_local_ips` to substitute a
 * LAN-reachable IP into the shown URL when they picked LAN mode — makes
 * the copy/scan flow actually work from a phone.
 */
export function SummaryStep({ draft, skipped }: SummaryStepProps) {
  const { t } = useTranslation()
  const [ips, setIps] = useState<string[]>([])
  const [copied, setCopied] = useState<"url" | "key" | null>(null)

  useEffect(() => {
    void (async () => {
      try {
        const list = await getTransport().call<string[]>("list_local_ips")
        if (Array.isArray(list)) setIps(list)
      } catch {
        setIps([])
      }
    })()
  }, [])

  const bindMode = draft.server?.bindMode ?? "local"
  const apiKey = draft.server?.apiKeyEnabled ? draft.server?.apiKey ?? "" : ""
  const host = bindMode === "lan" && ips[0] ? ips[0] : "localhost"
  const base = `http://${host}:8420`
  const fullUrl = apiKey ? `${base}/?token=${apiKey}` : `${base}/`

  async function copy(value: string, tag: "url" | "key") {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(tag)
      setTimeout(() => setCopied(null), 1500)
    } catch {
      /* ignore */
    }
  }

  async function openExternal() {
    try {
      await getTransport().call("open_url", { url: fullUrl })
    } catch {
      window.open(fullUrl, "_blank")
    }
  }

  const entries: Array<{ key: OnboardingStepKey; labelKey: string; value: string }> = [
    {
      key: "welcome",
      labelKey: "onboarding.summary.items.language",
      value: draft.language || "auto",
    },
    {
      key: "provider",
      labelKey: "onboarding.summary.items.provider",
      value: skipped.has("provider") ? "" : t("onboarding.summary.providerDone"),
    },
    {
      key: "profile",
      labelKey: "onboarding.summary.items.profile",
      value: [draft.profile?.name, draft.profile?.aiExperience].filter(Boolean).join(" · "),
    },
    {
      key: "personality",
      labelKey: "onboarding.summary.items.personality",
      value: draft.personalityPresetId
        ? t(`onboarding.personality.presets.${draft.personalityPresetId}.name`)
        : "",
    },
    {
      key: "safety",
      labelKey: "onboarding.summary.items.safety",
      value: draft.safety
        ? draft.safety.approvalsEnabled
          ? t("onboarding.summary.approvalsOn")
          : t("onboarding.summary.approvalsOff")
        : "",
    },
    {
      key: "skills",
      labelKey: "onboarding.summary.items.skills",
      value: draft.skills
        ? t("onboarding.summary.skillsDisabled", { n: draft.skills.disabled.length })
        : "",
    },
    {
      key: "server",
      labelKey: "onboarding.summary.items.server",
      value: bindMode === "lan" ? t("onboarding.server.lan") : t("onboarding.server.local"),
    },
    {
      key: "channels",
      labelKey: "onboarding.summary.items.channels",
      value: skipped.has("channels") ? "" : t("onboarding.summary.channelsDone"),
    },
  ]

  return (
    <div className="px-6 py-6 space-y-5 max-w-2xl mx-auto">
      <div className="flex flex-col items-center text-center gap-2">
        <div className="flex items-center justify-center h-14 w-14 rounded-2xl bg-primary/10 text-primary">
          <Sparkles className="h-7 w-7" />
        </div>
        <h2 className="text-xl font-semibold">{t("onboarding.summary.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.summary.subtitle")}</p>
      </div>

      <div className="grid gap-4 sm:grid-cols-[1fr_auto]">
        <ul className="rounded-lg border border-border divide-y divide-border">
          {entries.map((entry) => {
            const isSkipped = skipped.has(entry.key) || !entry.value
            return (
              <li
                key={entry.key}
                className="flex items-center justify-between gap-3 px-3 py-2 text-sm"
              >
                <span className="flex items-center gap-2">
                  {isSkipped ? (
                    <Minus className="h-4 w-4 text-muted-foreground/60" />
                  ) : (
                    <Check className="h-4 w-4 text-primary" />
                  )}
                  <span className="text-muted-foreground">{t(entry.labelKey)}</span>
                </span>
                <span
                  className={`text-right font-medium ${
                    isSkipped ? "text-muted-foreground/60" : ""
                  }`}
                >
                  {isSkipped ? t("onboarding.summary.skipped") : entry.value}
                </span>
              </li>
            )
          })}
        </ul>

        <div className="rounded-lg border border-border bg-muted/40 p-4 space-y-3 min-w-[240px]">
          <div>
            <div className="text-xs text-muted-foreground uppercase tracking-wider mb-1">
              {t("onboarding.summary.webUrlLabel")}
            </div>
            <code className="text-xs break-all block">{fullUrl}</code>
          </div>

          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={openExternal}>
              <ExternalLink className="h-3.5 w-3.5 mr-1" />
              {t("onboarding.summary.openWeb")}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => copy(fullUrl, "url")}>
              <Copy className="h-3.5 w-3.5 mr-1" />
              {copied === "url" ? t("onboarding.server.copied") : t("onboarding.server.copy")}
            </Button>
            {apiKey && (
              <Button variant="ghost" size="sm" onClick={() => copy(apiKey, "key")}>
                {copied === "key" ? t("onboarding.server.copied") : t("onboarding.summary.copyKey")}
              </Button>
            )}
          </div>
          <p className="text-[11px] text-muted-foreground leading-relaxed">
            {t("onboarding.summary.webHint")}
          </p>
        </div>
      </div>
    </div>
  )
}
