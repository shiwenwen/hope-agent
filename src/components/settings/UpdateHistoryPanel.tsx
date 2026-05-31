import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { CalendarDays, ExternalLink } from "lucide-react"
import { Button } from "@/components/ui/button"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { HOPE_AGENT_URLS, useAppVersion } from "@/lib/appMeta"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

const releaseNoteModules = import.meta.glob<string>("../../../docs/release-notes/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
})

interface ReleaseNoteEntry {
  version: string
  zh?: string
  en?: string
  date?: string
}

function compareVersionsDesc(a: string, b: string): number {
  const left = a.split(".").map((part) => Number(part) || 0)
  const right = b.split(".").map((part) => Number(part) || 0)
  const maxLen = Math.max(left.length, right.length)

  for (let i = 0; i < maxLen; i += 1) {
    const diff = (right[i] ?? 0) - (left[i] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
}

function extractDate(content: string): string | undefined {
  const match = content.match(/(?:发布日期|Release date)[：:]?\s*([0-9]{4}-[0-9]{2}-[0-9]{2})/i)
  return match?.[1]
}

function buildReleaseNotes(): ReleaseNoteEntry[] {
  const byVersion = new Map<string, ReleaseNoteEntry>()

  for (const [path, content] of Object.entries(releaseNoteModules)) {
    const match = path.match(/v([0-9]+(?:\.[0-9]+)+)(?:\.(en))?\.md$/)
    if (!match) continue

    const [, version, englishSuffix] = match
    const entry = byVersion.get(version) ?? { version }
    if (englishSuffix) {
      entry.en = content
    } else {
      entry.zh = content
    }
    entry.date ??= extractDate(content)
    byVersion.set(version, entry)
  }

  return Array.from(byVersion.values()).sort((a, b) => compareVersionsDesc(a.version, b.version))
}

async function openExternal(url: string) {
  try {
    await getTransport().call("open_url", { url })
  } catch {
    window.open(url, "_blank", "noopener,noreferrer")
  }
}

export default function UpdateHistoryPanel() {
  const { t, i18n } = useTranslation()
  const appVersion = useAppVersion()
  const releaseNotes = useMemo(() => buildReleaseNotes(), [])
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null)
  const activeRelease =
    releaseNotes.find((item) => item.version === selectedVersion) ?? releaseNotes[0] ?? null
  const preferChinese = i18n.language.startsWith("zh")
  const activeContent = activeRelease
    ? preferChinese
      ? activeRelease.zh ?? activeRelease.en
      : activeRelease.en ?? activeRelease.zh
    : null

  return (
    <div className="flex-1 overflow-hidden">
      <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4 p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-foreground">
              {t("about.updateHistory")}
            </h2>
            <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
              {t("about.updateHistoryDesc")}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="gap-1.5"
            onClick={() => openExternal(HOPE_AGENT_URLS.releases)}
          >
            {t("about.releases")}
            <ExternalLink className="h-3.5 w-3.5" />
          </Button>
        </div>

        {releaseNotes.length === 0 ? (
          <div className="flex flex-1 items-center justify-center rounded-2xl border border-border/70 bg-card text-sm text-muted-foreground">
            {t("about.updateHistoryEmpty")}
          </div>
        ) : (
          <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[230px_minmax(0,1fr)]">
            <aside className="min-h-0 overflow-y-auto rounded-2xl border border-border/70 bg-card p-2">
              <div className="space-y-1">
                {releaseNotes.map((item, index) => {
                  const isActive = item.version === activeRelease?.version
                  const isCurrent = item.version === appVersion

                  return (
                    <button
                      key={item.version}
                      type="button"
                      className={cn(
                        "flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left transition-colors",
                        isActive
                          ? "bg-secondary text-foreground"
                          : "text-muted-foreground hover:bg-secondary/60 hover:text-foreground",
                      )}
                      onClick={() => setSelectedVersion(item.version)}
                    >
                      <span className="flex-1">
                        <span className="block text-sm font-medium">v{item.version}</span>
                        <span className="mt-0.5 block text-[11px] text-muted-foreground">
                          {item.date ??
                            t("about.updateHistoryUndated")}
                        </span>
                      </span>
                      {index === 0 && (
                        <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                          {t("about.updateHistoryLatest")}
                        </span>
                      )}
                      {isCurrent && (
                        <span className="h-2 w-2 rounded-full bg-emerald-500" aria-hidden="true" />
                      )}
                    </button>
                  )
                })}
              </div>
            </aside>

            <section className="min-h-0 overflow-y-auto rounded-2xl border border-border/70 bg-card">
              {activeRelease && (
                <div className="sticky top-0 z-10 flex flex-wrap items-center justify-between gap-3 border-b border-border/70 bg-card/95 px-5 py-3 backdrop-blur">
                  <div>
                    <h3 className="text-base font-semibold text-foreground">
                      Hope Agent v{activeRelease.version}
                    </h3>
                    {activeRelease.date && (
                      <div className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                        <CalendarDays className="h-3.5 w-3.5" />
                        {activeRelease.date}
                      </div>
                    )}
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="gap-1.5"
                    onClick={() =>
                      openExternal(`${HOPE_AGENT_URLS.releases}/tag/v${activeRelease.version}`)
                    }
                  >
                    GitHub
                    <ExternalLink className="h-3.5 w-3.5" />
                  </Button>
                </div>
              )}
              <div className="update-notes-markdown px-5 py-4 text-sm leading-6 text-muted-foreground">
                {activeContent ? (
                  <MarkdownRenderer content={activeContent} />
                ) : (
                  <p>
                    {t("about.updateHistoryNoNotes")}
                  </p>
                )}
              </div>
            </section>
          </div>
        )}
      </div>
    </div>
  )
}
