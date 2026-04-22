import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import { Brain, ExternalLink, Globe, Monitor } from "lucide-react"
import logoUrl from "@/assets/logo.png"
import { Button } from "@/components/ui/button"
import { APP_VERSION, HOPE_AGENT_URLS } from "@/lib/appMeta"
import { getTransport } from "@/lib/transport-provider"

interface HighlightItem {
  icon: LucideIcon
  title: string
  description: string
  cardClass: string
  iconClass: string
}

export default function AboutPanel() {
  const { t } = useTranslation()

  const highlights: HighlightItem[] = [
    {
      icon: Monitor,
      title: t("about.featureDailyTitle"),
      description: t("about.featureDailyDesc"),
      cardClass: "border-border/70 bg-sky-500/6",
      iconClass: "border border-sky-500/15 bg-sky-500/10 text-sky-600 dark:text-sky-300",
    },
    {
      icon: Brain,
      title: t("about.featureMemoryTitle"),
      description: t("about.featureMemoryDesc"),
      cardClass: "border-border/70 bg-emerald-500/6",
      iconClass:
        "border border-emerald-500/15 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
    },
    {
      icon: Globe,
      title: t("about.featureReachTitle"),
      description: t("about.featureReachDesc"),
      cardClass: "border-border/70 bg-amber-500/6",
      iconClass: "border border-amber-500/15 bg-amber-500/10 text-amber-600 dark:text-amber-300",
    },
  ]

  async function openExternal(url: string) {
    try {
      await getTransport().call("open_url", { url })
    } catch {
      window.open(url, "_blank", "noopener,noreferrer")
    }
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-6">
        <section className="rounded-[28px] border border-border/70 bg-card px-6 py-7 lg:px-8 lg:py-8">
          <div>
            <div className="inline-flex w-fit items-center gap-2 rounded-full border border-border/70 bg-secondary/40 px-3 py-1 text-[11px] font-medium uppercase tracking-[0.22em] text-primary/80">
              <span className="h-1.5 w-1.5 rounded-full bg-primary" />
              {t("about.badge")}
            </div>

            <div className="mt-5 flex items-center gap-4">
              <div className="flex h-20 w-20 items-center justify-center rounded-[22px] border border-border/70 bg-secondary/30 p-2">
                <img
                  src={logoUrl}
                  alt="Hope Agent"
                  className="h-full w-full rounded-[18px] object-cover"
                  draggable={false}
                />
              </div>
              <div className="min-w-0">
                <h2 className="text-3xl font-semibold tracking-tight text-foreground lg:text-4xl">
                  Hope Agent
                </h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  {t("about.version")} v{APP_VERSION}
                </p>
              </div>
            </div>

            <p className="mt-6 max-w-4xl text-2xl font-semibold leading-tight tracking-tight text-foreground lg:text-4xl">
              {t("about.tagline")}
            </p>
            <p className="mt-4 max-w-3xl text-sm leading-7 text-muted-foreground lg:text-base">
              {t("about.description")}
            </p>

            <div className="mt-6 flex flex-wrap gap-3">
              <Button variant="outline" onClick={() => openExternal(HOPE_AGENT_URLS.github)}>
                {t("about.github")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
              <Button variant="secondary" onClick={() => openExternal(HOPE_AGENT_URLS.releases)}>
                {t("about.releases")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
              <Button variant="ghost" onClick={() => openExternal(HOPE_AGENT_URLS.feedback)}>
                {t("about.feedback")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
            </div>
          </div>
        </section>

        <section className="grid gap-4 lg:grid-cols-3">
          {highlights.map((item) => {
            const Icon = item.icon
            return (
              <div key={item.title} className={`rounded-[24px] border p-5 ${item.cardClass}`}>
                <div
                  className={`flex h-11 w-11 items-center justify-center rounded-2xl ${item.iconClass}`}
                >
                  <Icon className="h-5 w-5" />
                </div>
                <h3 className="mt-5 text-lg font-semibold tracking-tight text-foreground">
                  {item.title}
                </h3>
                <p className="mt-2 text-sm leading-6 text-muted-foreground">{item.description}</p>
              </div>
            )
          })}
        </section>

        <section className="rounded-[24px] border border-border/70 bg-secondary/25 px-6 py-5">
          <p className="max-w-4xl text-sm leading-7 text-muted-foreground lg:text-base">
            {t("about.closing")}
          </p>
        </section>
      </div>
    </div>
  )
}
