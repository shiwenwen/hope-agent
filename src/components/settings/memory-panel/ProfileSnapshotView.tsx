import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { Button } from "@/components/ui/button"
import {
  ExternalLink,
  FileText,
  Globe,
  Loader2,
  MessageSquare,
  Sparkles,
  UserCircle,
} from "lucide-react"
import { requestChatFocus } from "@/components/chat/chatFocus"
import type { AgentInfo } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import { requestMemoryScopeFocus } from "./scopeFocus"
import { requestMemoryFocus } from "./memoryFocus"
import { profileSnapshotOperationErrorToast } from "./profileSnapshotOperationFeedback"

interface ProfileSnapshotSourceRecord {
  lineIndex?: number | null
  claimId: string
  claimType: string
  content: string
  confidence: number
  salience: number
  evidenceId?: string | null
  evidenceClass?: string | null
  evidenceSourceType?: string | null
  evidenceQuote?: string | null
  evidenceSessionId?: string | null
  evidenceMessageId?: string | null
  evidenceFilePath?: string | null
  evidenceUrl?: string | null
}

// Mirrors ha-core `ProfileSnapshotRecord` (camelCase).
interface ProfileSnapshotRecord {
  scopeType: string
  scopeId?: string | null
  version: number
  bodyMd: string
  sources?: ProfileSnapshotSourceRecord[]
  sourceRunId: string
  createdAt: string
}

// Mirrors ha-core `ProfileReport` (camelCase).
interface ProfileReport {
  runId?: string | null
  scanned: number
  scopes: number
  snapshotsWritten: number
  durationMs: number
  note?: string | null
}

function profileLines(bodyMd: string): string[] {
  return bodyMd
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
}

function pct(value: number): string {
  if (!Number.isFinite(value)) return "?"
  return `${Math.round(value * 100)}%`
}

function parseEvidenceMessageId(messageId?: string | null): number | undefined {
  if (!messageId) return undefined
  const parsed = Number(messageId)
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined
}

function hasEvidenceSource(source: ProfileSnapshotSourceRecord): boolean {
  return Boolean(source.evidenceSessionId || source.evidenceFilePath || source.evidenceUrl)
}

function evidenceSourceLabel(source: ProfileSnapshotSourceRecord, t: (key: string) => string) {
  if (source.evidenceSessionId) return t("settings.claims.openChatSource")
  if (source.evidenceFilePath) return t("settings.claims.openFileSource")
  if (source.evidenceUrl) return t("settings.claims.openUrlSource")
  return t("chat.memoryTrace.openSource")
}

function EvidenceSourceIcon({ source }: { source: ProfileSnapshotSourceRecord }) {
  if (source.evidenceSessionId) return <MessageSquare className="h-3 w-3" />
  if (source.evidenceFilePath) return <FileText className="h-3 w-3" />
  if (source.evidenceUrl) return <Globe className="h-3 w-3" />
  return <ExternalLink className="h-3 w-3" />
}

function ProfileSourceButton({
  source,
  onOpenClaim,
  muted = false,
}: {
  source: ProfileSnapshotSourceRecord
  onOpenClaim: (claimId: string) => void
  muted?: boolean
}) {
  const { t } = useTranslation()
  const evidenceQuote = source.evidenceQuote?.trim()
  const title = evidenceQuote ? `${source.content}\n\n${evidenceQuote}` : source.content
  const className = muted
    ? "inline-flex max-w-72 flex-col items-start gap-0.5 rounded border border-border/60 bg-background/70 px-1.5 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
    : "inline-flex max-w-72 flex-col items-start gap-0.5 rounded border border-primary/15 bg-primary/6 px-1.5 py-0.5 text-[10px] text-primary transition-colors hover:bg-primary/10"
  const metaClassName = muted ? "text-muted-foreground/80" : "text-primary/70"
  const quoteClassName = muted
    ? "max-w-full whitespace-normal break-words text-left text-[10px] leading-snug text-muted-foreground/80"
    : "max-w-full whitespace-normal break-words text-left text-[10px] leading-snug text-primary/75"
  const canOpenEvidenceSource = hasEvidenceSource(source)
  const openEvidenceSource = () => {
    if (source.evidenceSessionId) {
      requestChatFocus({
        sessionId: source.evidenceSessionId,
        targetMessageId: parseEvidenceMessageId(source.evidenceMessageId),
      })
      return
    }
    if (source.evidenceFilePath) {
      void getTransport()
        .openFilePath(source.evidenceFilePath, {
          sessionId: source.evidenceSessionId ?? undefined,
        })
        .catch((e) => {
          logger.warn(
            "settings",
            "ProfileSnapshotView::openEvidenceFile",
            "Failed to open profile evidence file",
            e,
          )
          const failureToast = profileSnapshotOperationErrorToast("openEvidenceSource", t, e)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
        })
      return
    }
    if (source.evidenceUrl) {
      openExternalUrl(source.evidenceUrl, {
        onError: (e) => {
          logger.warn(
            "settings",
            "ProfileSnapshotView::openEvidenceUrl",
            "Failed to open profile evidence URL",
            e,
          )
          const failureToast = profileSnapshotOperationErrorToast("openEvidenceSource", t, e)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
        },
      })
    }
  }

  return (
    <span className="inline-flex max-w-full items-stretch gap-1">
      <button
        type="button"
        onClick={() => onOpenClaim(source.claimId)}
        className={className}
        data-ha-title-tip={title}
      >
        <span className="flex max-w-full items-center gap-1">
          <ExternalLink className="h-3 w-3 shrink-0" />
          <span className="truncate">{source.claimType}</span>
          <span className={metaClassName}>
            {t("settings.profile.sourceConfidence", {
              value: pct(source.confidence),
            })}
          </span>
        </span>
        {evidenceQuote && (
          <span className={quoteClassName}>
            {t("settings.profile.sourceEvidence", { quote: evidenceQuote })}
          </span>
        )}
      </button>
      {canOpenEvidenceSource && (
        <button
          type="button"
          onClick={openEvidenceSource}
          className="inline-flex w-6 shrink-0 items-center justify-center rounded border border-border/60 bg-background/70 text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
          data-ha-title-tip={evidenceSourceLabel(source, t)}
          aria-label={evidenceSourceLabel(source, t)}
        >
          <EvidenceSourceIcon source={source} />
        </button>
      )}
    </span>
  )
}

/**
 * Read-only Memory Profile view (next-gen Dreaming Phase 4). Shows the latest
 * synthesised profile snapshot per scope (global / agent / project) via
 * `dreaming_list_profile_snapshots`, and a manual "refresh" that runs an
 * LLM-rewrite synthesis cycle (`dreaming_run_profile`). The profile is
 * grounded in active claims — editing / rejecting lands with the correction
 * loop in a later PR.
 */
export default function ProfileSnapshotView() {
  const { t } = useTranslation()
  const [snapshots, setSnapshots] = useState<ProfileSnapshotRecord[]>([])
  const [loading, setLoading] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [agentNames, setAgentNames] = useState<Map<string, string>>(() => new Map())
  const [projectNames, setProjectNames] = useState<Map<string, string>>(() => new Map())

  const scopeLabel = useCallback(
    (r: { scopeType: string; scopeId?: string | null }) => {
      if (r.scopeType === "global") return t("dashboard.dreaming.review.scopeGlobal")
      const id = r.scopeId ?? "?"
      if (r.scopeType === "agent") {
        const name = r.scopeId ? agentNames.get(r.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeAgent")}: ${name}` : `agent:${id}`
      }
      if (r.scopeType === "project") {
        const name = r.scopeId ? projectNames.get(r.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeProject")}: ${name}` : `project:${id}`
      }
      return `${r.scopeType}:${id}`
    },
    [agentNames, projectNames, t],
  )

  const openScope = (snapshot: ProfileSnapshotRecord) => {
    if (
      (snapshot.scopeType === "agent" || snapshot.scopeType === "project") &&
      snapshot.scopeId
    ) {
      requestMemoryScopeFocus({ kind: snapshot.scopeType, id: snapshot.scopeId })
    }
  }

  const openClaim = useCallback((claimId: string) => {
    requestMemoryFocus({ kind: "claim", id: claimId })
  }, [])

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const list = await getTransport().call<ProfileSnapshotRecord[]>(
        "dreaming_list_profile_snapshots",
      )
      setSnapshots(list ?? [])
    } catch (e) {
      logger.error("settings", "ProfileSnapshotView::list", "Failed to list snapshots", e)
      setSnapshots([])
    } finally {
      setLoading(false)
    }
  }, [])

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      const r = await getTransport().call<ProfileReport>("dreaming_run_profile")
      await load()
      if (r?.runId) {
        toast.success(
          t("settings.profile.refreshDone", { count: r?.snapshotsWritten ?? 0 }),
        )
      } else {
        // Skipped before a run row was created (disabled / lock contention).
        toast.message(t("settings.profile.refreshSkipped"), {
          description: r?.note ?? undefined,
        })
      }
    } catch (e) {
      logger.error("settings", "ProfileSnapshotView::refresh", "Failed to run synthesis", e)
      const failureToast = profileSnapshotOperationErrorToast("refresh", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRefreshing(false)
    }
  }, [t, load])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    let cancelled = false
    const tx = getTransport()
    void Promise.allSettled([
      tx.call<AgentInfo[]>("list_agents"),
      tx.call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true }),
    ]).then(([agentsResult, projectsResult]) => {
      if (cancelled) return
      if (agentsResult.status === "fulfilled") {
        setAgentNames(new Map((agentsResult.value ?? []).map((agent) => [agent.id, agent.name])))
      }
      if (projectsResult.status === "fulfilled") {
        setProjectNames(
          new Map((projectsResult.value ?? []).map((project) => [project.id, project.name])),
        )
      }
    })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    return getTransport().listen("dreaming:cycle_complete", (raw) => {
      const payload = raw as { phase?: string }
      if (payload.phase === "profile") void load()
    })
  }, [load])

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-3">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-sm font-medium">{t("settings.profile.title")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.profile.desc")}</div>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1.5 text-xs"
          onClick={refresh}
          disabled={refreshing}
        >
          {refreshing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
          {refreshing ? t("settings.profile.refreshing") : t("settings.profile.refresh")}
        </Button>
      </div>

      {loading ? (
        <div className="px-3 py-10 text-xs text-muted-foreground text-center inline-flex items-center gap-1.5 w-full justify-center">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("common.loading")}
        </div>
      ) : snapshots.length === 0 ? (
        <div className="rounded-lg border border-border/60 px-4 py-10 text-center text-xs text-muted-foreground">
          <UserCircle className="h-6 w-6 mx-auto mb-2 opacity-40" />
          <div>{t("settings.profile.empty")}</div>
          <div className="mt-1">{t("settings.profile.emptyHint")}</div>
        </div>
      ) : (
        <div className="space-y-3">
          {snapshots.map((s) => (
            <ProfileSnapshotCard
              key={`${s.scopeType}:${s.scopeId ?? ""}`}
              snapshot={s}
              scopeLabel={scopeLabel(s)}
              onOpenScope={() => openScope(s)}
              onOpenClaim={openClaim}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function ProfileSnapshotCard({
  snapshot,
  scopeLabel,
  onOpenScope,
  onOpenClaim,
}: {
  snapshot: ProfileSnapshotRecord
  scopeLabel: string
  onOpenScope: () => void
  onOpenClaim: (claimId: string) => void
}) {
  const { t } = useTranslation()
  const lines = useMemo(() => profileLines(snapshot.bodyMd), [snapshot.bodyMd])
  const sources = snapshot.sources ?? []
  const lineSources = useMemo(() => {
    const byLine = new Map<number, ProfileSnapshotSourceRecord[]>()
    for (const source of sources) {
      if (typeof source.lineIndex !== "number") continue
      const list = byLine.get(source.lineIndex) ?? []
      list.push(source)
      byLine.set(source.lineIndex, list)
    }
    return byLine
  }, [sources])
  const scopeSources = sources.filter((source) => typeof source.lineIndex !== "number")

  return (
    <div className="overflow-hidden rounded-lg border border-border/60">
      <div className="flex items-center justify-between gap-2 border-b border-border/60 bg-secondary/20 px-3 py-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate text-xs font-medium">{scopeLabel}</span>
          {(snapshot.scopeType === "agent" || snapshot.scopeType === "project") &&
            snapshot.scopeId && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                onClick={onOpenScope}
                data-ha-title-tip={t("settings.claims.openScope")}
                aria-label={t("settings.claims.openScope")}
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </Button>
            )}
        </div>
        <span className="text-[10px] text-muted-foreground">
          {t("settings.profile.version", { version: snapshot.version })} · {snapshot.createdAt}
        </span>
      </div>

      <div className="space-y-2 px-3 py-2 text-xs">
        {lines.map((line, index) => {
          const claimSources = lineSources.get(index) ?? []
          return (
            <div key={`${index}:${line}`} className="rounded-md bg-background/40 px-2 py-1.5">
              <div className="whitespace-pre-wrap break-words text-foreground/90">{line}</div>
              {claimSources.length > 0 && (
                <div className="mt-1.5 flex flex-wrap gap-1.5">
                  {claimSources.map((source) => (
                    <ProfileSourceButton
                      key={`${source.claimId}:${index}`}
                      source={source}
                      onOpenClaim={onOpenClaim}
                    />
                  ))}
                </div>
              )}
            </div>
          )
        })}

        {scopeSources.length > 0 && (
          <div className="rounded-md border border-border/50 bg-muted/30 px-2 py-1.5">
            <div className="text-[10px] font-medium uppercase text-muted-foreground">
              {t("settings.profile.scopeSources")}
            </div>
            <div className="mt-1 flex flex-wrap gap-1.5">
              {scopeSources.map((source) => (
                <ProfileSourceButton
                  key={source.claimId}
                  source={source}
                  onOpenClaim={onOpenClaim}
                  muted
                />
              ))}
            </div>
          </div>
        )}

        {sources.length === 0 && (
          <div className="rounded-md border border-dashed border-border/60 px-2 py-1.5 text-[10px] text-muted-foreground">
            {t("settings.profile.noSources")}
          </div>
        )}
      </div>
    </div>
  )
}
