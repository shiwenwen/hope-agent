import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Copy,
  Cpu,
  ExternalLink,
  Globe as GlobeIcon,
  Key as KeyIcon,
  Languages,
  MessageSquare,
  Puzzle,
  Server as ServerIcon,
  Shield,
  Smile,
  User as UserIcon,
  type LucideIcon,
} from "lucide-react"

import alphaLogoUrl from "@/assets/alpha-logo.png"
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

  const entries: Array<{
    key: OnboardingStepKey
    labelKey: string
    value: string
    icon: LucideIcon
  }> = [
    {
      key: "welcome",
      labelKey: "onboarding.summary.items.language",
      value: draft.language || "auto",
      icon: Languages,
    },
    {
      key: "provider",
      labelKey: "onboarding.summary.items.provider",
      value: skipped.has("provider") ? "" : t("onboarding.summary.providerDone"),
      icon: Cpu,
    },
    {
      key: "profile",
      labelKey: "onboarding.summary.items.profile",
      value: [draft.profile?.name, draft.profile?.aiExperience].filter(Boolean).join(" · "),
      icon: UserIcon,
    },
    {
      key: "personality",
      labelKey: "onboarding.summary.items.personality",
      value: draft.personalityPresetId
        ? t(`onboarding.personality.presets.${draft.personalityPresetId}.name`)
        : "",
      icon: Smile,
    },
    {
      key: "safety",
      labelKey: "onboarding.summary.items.safety",
      value: draft.safety
        ? draft.safety.approvalsEnabled
          ? t("onboarding.summary.approvalsOn")
          : t("onboarding.summary.approvalsOff")
        : "",
      icon: Shield,
    },
    {
      key: "skills",
      labelKey: "onboarding.summary.items.skills",
      value: draft.skills
        ? t("onboarding.summary.skillsDisabled", { n: draft.skills.disabled.length })
        : "",
      icon: Puzzle,
    },
    {
      key: "server",
      labelKey: "onboarding.summary.items.server",
      value: bindMode === "lan" ? t("onboarding.server.lan") : t("onboarding.server.local"),
      icon: ServerIcon,
    },
    {
      key: "channels",
      labelKey: "onboarding.summary.items.channels",
      value: skipped.has("channels") ? "" : t("onboarding.summary.channelsDone"),
      icon: MessageSquare,
    },
  ]

  return (
    <div className="px-6 py-6 space-y-6 max-w-2xl mx-auto">
      {/* Hero */}
      <div className="flex flex-col items-center text-center gap-3">
        <img
          src={alphaLogoUrl}
          alt="Hope Agent"
          className="h-20 w-20 object-contain"
          draggable={false}
        />
        <h2 className="text-2xl font-semibold tracking-tight">
          {t("onboarding.summary.title")}
        </h2>
        <p className="text-sm text-muted-foreground max-w-md leading-relaxed">
          {t("onboarding.summary.subtitle")}
        </p>
      </div>

      {/* Summary chips — two-column on ≥sm, each chip is self-contained */}
      <div className="grid gap-2 sm:grid-cols-2">
        {entries.map((entry) => {
          const isSkipped = skipped.has(entry.key) || !entry.value
          const Icon = entry.icon
          return (
            <div
              key={entry.key}
              className={`flex items-center gap-3 rounded-lg border px-3 py-2.5 text-sm transition-colors ${
                isSkipped
                  ? "border-border/60 bg-muted/20"
                  : "border-border bg-card"
              }`}
            >
              <div
                className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-md ${
                  isSkipped
                    ? "bg-muted text-muted-foreground/60"
                    : "bg-primary/10 text-primary"
                }`}
              >
                <Icon className="h-4 w-4" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[11px] uppercase tracking-wider text-muted-foreground/80">
                  {t(entry.labelKey)}
                </div>
                <div
                  className={`text-sm font-medium truncate ${
                    isSkipped ? "text-muted-foreground/60 italic" : ""
                  }`}
                >
                  {isSkipped ? t("onboarding.summary.skipped") : entry.value}
                </div>
              </div>
            </div>
          )
        })}
      </div>

      {/* Web GUI hero card */}
      <div className="rounded-xl border border-primary/20 bg-gradient-to-br from-primary/5 to-transparent p-5 space-y-4">
        <div className="flex items-center gap-2">
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <GlobeIcon className="h-4 w-4" />
          </div>
          <div className="flex-1">
            <div className="text-sm font-semibold">
              {t("onboarding.summary.webUrlLabel")}
            </div>
            <p className="text-xs text-muted-foreground leading-snug">
              {t("onboarding.summary.webHint")}
            </p>
          </div>
        </div>

        <code className="block rounded-md border border-border bg-background/60 px-3 py-2 text-xs break-all font-mono">
          {fullUrl}
        </code>

        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={openExternal}>
            <ExternalLink className="h-3.5 w-3.5 mr-1" />
            {t("onboarding.summary.openWeb")}
          </Button>
          <Button variant="outline" size="sm" onClick={() => copy(fullUrl, "url")}>
            <Copy className="h-3.5 w-3.5 mr-1" />
            {copied === "url" ? t("onboarding.server.copied") : t("onboarding.server.copy")}
          </Button>
          {apiKey && (
            <Button variant="outline" size="sm" onClick={() => copy(apiKey, "key")}>
              <KeyIcon className="h-3.5 w-3.5 mr-1" />
              {copied === "key" ? t("onboarding.server.copied") : t("onboarding.summary.copyKey")}
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
