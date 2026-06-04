import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { CalendarDays, ExternalLink } from "lucide-react"
import { Button } from "@/components/ui/button"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { HOPE_AGENT_URLS, useAppVersion } from "@/lib/appMeta"
import { logger } from "@/lib/logger"
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
  remoteBody?: string
  htmlUrl?: string
}

interface GitHubReleaseEntry {
  version: string
  date?: string
  body?: string
  htmlUrl?: string
}

interface ParsedVersion {
  core: number[]
  prerelease: string | null
}

const GITHUB_RELEASES_API =
  "https://api.github.com/repos/shiwenwen/hope-agent/releases?per_page=100"
const RELEASE_TAG_RE = /^v?([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)$/

function parseVersion(version: string): ParsedVersion | null {
  const [withoutBuild] = version.split("+")
  const [corePart, prerelease = null] = withoutBuild.split("-", 2)
  if (!corePart) return null
  const coreParts = corePart.split(".")
  if (coreParts.length === 0 || coreParts.some((part) => !/^[0-9]+$/.test(part))) return null
  const core = coreParts.map((part) => Number(part))
  return { core, prerelease }
}

function compareVersionsDesc(a: string, b: string): number {
  const left = parseVersion(a)
  const right = parseVersion(b)

  if (!left || !right) return b.localeCompare(a, undefined, { numeric: true })
  const maxLen = Math.max(left.core.length, right.core.length)

  for (let i = 0; i < maxLen; i += 1) {
    const diff = (right.core[i] ?? 0) - (left.core[i] ?? 0)
    if (diff !== 0) return diff
  }

  if (left.prerelease === right.prerelease) return 0
  if (!left.prerelease) return -1
  if (!right.prerelease) return 1
  return right.prerelease.localeCompare(left.prerelease, undefined, { numeric: true })
}

function normalizeReleaseVersion(tagName: string): string | null {
  return tagName.trim().match(RELEASE_TAG_RE)?.[1] ?? null
}

function extractDate(content: string): string | undefined {
  const match = content.match(/(?:发布日期|Release date)[：:]?\s*([0-9]{4}-[0-9]{2}-[0-9]{2})/i)
  return match?.[1]
}

function buildReleaseNotes(): ReleaseNoteEntry[] {
  const byVersion = new Map<string, ReleaseNoteEntry>()

  for (const [path, content] of Object.entries(releaseNoteModules)) {
    const match = path.match(/\/v([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)(?:\.(en))?\.md$/)
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function readStringField(value: Record<string, unknown>, key: string): string | undefined {
  const raw = value[key]
  return typeof raw === "string" && raw.trim() ? raw : undefined
}

function dateFromPublishedAt(value: string | undefined): string | undefined {
  if (!value) return undefined
  const date = value.slice(0, 10)
  return /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/.test(date) ? date : undefined
}

function toGitHubReleaseEntry(value: unknown): GitHubReleaseEntry | null {
  if (!isRecord(value) || value.draft === true) return null

  const version = normalizeReleaseVersion(readStringField(value, "tag_name") ?? "")
  if (!version) return null

  return {
    version,
    date: dateFromPublishedAt(readStringField(value, "published_at")),
    body: readStringField(value, "body"),
    htmlUrl: readStringField(value, "html_url"),
  }
}

async function fetchGitHubReleasePage(
  page: number,
  signal: AbortSignal,
): Promise<{ entries: GitHubReleaseEntry[]; rawCount: number }> {
  const url = new URL(GITHUB_RELEASES_API)
  url.searchParams.set("page", String(page))
  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.github+json",
    },
    signal,
  })

  if (!response.ok) {
    throw new Error(`GitHub releases request failed: ${response.status} ${response.statusText}`)
  }

  const raw: unknown = await response.json()
  if (!Array.isArray(raw)) throw new Error("GitHub releases response was not an array")

  return {
    entries: raw.flatMap((item) => {
      const entry = toGitHubReleaseEntry(item)
      return entry ? [entry] : []
    }),
    rawCount: raw.length,
  }
}

async function fetchGitHubReleases(signal: AbortSignal): Promise<GitHubReleaseEntry[]> {
  const entries: GitHubReleaseEntry[] = []

  for (let page = 1; page <= 10; page += 1) {
    const result = await fetchGitHubReleasePage(page, signal)
    entries.push(...result.entries)
    if (result.rawCount < 100) break
  }

  return entries
}

function mergeReleaseNotes(
  localNotes: ReleaseNoteEntry[],
  remoteReleases: GitHubReleaseEntry[],
): ReleaseNoteEntry[] {
  const byVersion = new Map<string, ReleaseNoteEntry>()

  for (const note of localNotes) {
    byVersion.set(note.version, { ...note })
  }

  for (const release of remoteReleases) {
    const existing = byVersion.get(release.version)
    byVersion.set(release.version, {
      ...existing,
      version: release.version,
      date: existing?.date ?? release.date,
      zh: existing?.zh,
      en: existing?.en,
      remoteBody: existing?.remoteBody ?? release.body,
      htmlUrl: existing?.htmlUrl ?? release.htmlUrl,
    })
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
  const normalizedAppVersion = normalizeReleaseVersion(appVersion) ?? appVersion
  const localReleaseNotes = useMemo(() => buildReleaseNotes(), [])
  const [remoteReleases, setRemoteReleases] = useState<GitHubReleaseEntry[]>([])
  const releaseNotes = useMemo(
    () => mergeReleaseNotes(localReleaseNotes, remoteReleases),
    [localReleaseNotes, remoteReleases],
  )
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null)
  const activeRelease =
    releaseNotes.find((item) => item.version === selectedVersion) ?? releaseNotes[0] ?? null
  const preferChinese = i18n.language.startsWith("zh")
  const activeContent = activeRelease
    ? preferChinese
      ? (activeRelease.zh ?? activeRelease.en ?? activeRelease.remoteBody)
      : (activeRelease.en ?? activeRelease.zh ?? activeRelease.remoteBody)
    : null

  useEffect(() => {
    const controller = new AbortController()

    fetchGitHubReleases(controller.signal)
      .then((entries) => {
        if (controller.signal.aborted) return
        setRemoteReleases(entries)
      })
      .catch((e: unknown) => {
        if (controller.signal.aborted) return
        logger.warn("settings", "UpdateHistoryPanel::fetchGitHubReleases", "Failed to load", e)
      })

    return () => {
      controller.abort()
    }
  }, [])

  return (
    <div className="flex-1 overflow-hidden">
      <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4 p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-foreground">{t("about.updateHistory")}</h2>
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
                  const isCurrent = item.version === normalizedAppVersion

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
                          {item.date ?? t("about.updateHistoryUndated")}
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
                      openExternal(
                        activeRelease.htmlUrl ??
                          `${HOPE_AGENT_URLS.releases}/tag/v${activeRelease.version}`,
                      )
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
                  <p>{t("about.updateHistoryNoNotes")}</p>
                )}
              </div>
            </section>
          </div>
        )}
      </div>
    </div>
  )
}
