import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Bot, ChevronLeft, ChevronRight, RefreshCw, X, XCircle } from "lucide-react"

import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { AgentAvatarBadge } from "@/components/common/AgentSelectDisplay"
import type { AgentSummaryForSidebar, SessionMeta, SubagentRun } from "@/types/chat"
import { formatDuration } from "../chatUtils"
import { formatModelLabel, TERMINAL_STATUSES, useAgentsMap } from "../subagentShared"
import { SubagentRunRow, SubagentStatusBadge } from "./SubagentRunRow"
import SubagentRunDetails from "../SubagentRunDetails"
import SubagentSessionView from "./SubagentSessionView"
import { PANEL_SCROLL_FADE } from "../right-panel/panelFade"
import { useSubagentRunDetail, type SubagentRunsSnapshot } from "./useSubagentRuns"
import type { SubagentOpenTarget } from "./subagentRunModel"

export interface SubagentPanelSelectRequest {
  runId?: string
  childSessionId?: string
  /** Monotonic token so re-clicking the same run re-focuses it. */
  nonce: number
}

interface SubagentPanelProps {
  sessionId: string | null
  runsState: SubagentRunsSnapshot
  agents: AgentSummaryForSidebar[]
  selectRequest: SubagentPanelSelectRequest | null
  onClose: () => void
}

interface NavEntry {
  runId: string
  sessionId: string
  agentId: string
  label?: string
}

type DetailPane = "result" | "conversation" | "details"

/** Segmented-control tab, mirroring the flat shared TabsTrigger look. */
function PaneTab({
  active,
  label,
  onSelect,
}: {
  active: boolean
  label: string
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={active}
      className={cn(
        "inline-flex shrink-0 items-center whitespace-nowrap rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors",
        active ? "bg-background text-foreground" : "hover:text-foreground",
      )}
    >
      {label}
    </button>
  )
}

function GroupHeading({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 pb-1 pt-2 text-[11px] font-medium text-muted-foreground">{children}</div>
  )
}

/**
 * Master → detail panel for a session's sub-agent runs. The list and the detail
 * each own the whole panel (a right panel is too narrow to split vertically);
 * `stack` doubles as the router — empty renders the list, non-empty renders that
 * run, and deeper entries are nested drill-downs reached from chips inside a
 * transcript.
 */
export default function SubagentPanel({
  sessionId,
  runsState,
  agents,
  selectRequest,
  onClose,
}: SubagentPanelProps) {
  const { t } = useTranslation()
  const agentsMap = useAgentsMap()
  const { runs, byId, byChildSessionId, runningCount, loaded } = runsState
  const finishedCount = runs.length - runningCount

  const [stack, setStack] = useState<NavEntry[]>([])
  const [reloadToken, setReloadToken] = useState(0)
  // `null` = follow the status-derived default pane; a value = the user picked.
  const [paneOverride, setPaneOverride] = useState<DetailPane | null>(null)
  const [cancelling, setCancelling] = useState(false)
  // Child-session meta, captured from the transcript view that already loads it
  // — carries the resolved provider name + Think level that `modelUsed` can't.
  // Tagged with its session so a stale one is never shown against another run.
  const [childMeta, setChildMeta] = useState<{
    sessionId: string
    meta: SessionMeta | null
  } | null>(null)
  const handledNonceRef = useRef<number | null>(null)

  const top = stack[stack.length - 1] ?? null
  const selectedRunId = top?.runId || null
  const primaryRun = selectedRunId ? byId.get(selectedRunId) : undefined
  const detail = useSubagentRunDetail(selectedRunId, primaryRun, reloadToken)

  const status = detail?.status ?? "spawning"
  const isTerminal = TERMINAL_STATUSES.has(status)
  // Finished run → lead with what it produced. Live run → the transcript is the
  // story and there is no result yet. Re-derives if it completes while open,
  // unless the user has already picked a pane.
  const pane: DetailPane =
    paneOverride ?? (isTerminal && detail?.result ? "result" : "conversation")

  const [runningRuns, finishedRuns] = useMemo(() => {
    const live: SubagentRun[] = []
    const done: SubagentRun[] = []
    for (const run of runs) (TERMINAL_STATUSES.has(run.status) ? done : live).push(run)
    return [live, done]
  }, [runs])

  const entryTitle = useCallback(
    (entry: NavEntry) =>
      entry.label ||
      agentsMap.get(entry.agentId)?.name ||
      entry.agentId ||
      entry.sessionId.slice(0, 8),
    [agentsMap],
  )

  const resetDisclosures = useCallback(() => {
    setPaneOverride(null)
  }, [])

  const selectRun = useCallback(
    (run: SubagentRun) => {
      setStack([
        {
          runId: run.runId,
          sessionId: run.childSessionId,
          agentId: run.childAgentId,
          label: run.label,
        },
      ])
      resetDisclosures()
    },
    [resetDisclosures],
  )

  // Resolve an incoming chip / workspace-row request into a selection. Re-runs
  // when the snapshot maps update, so a request that arrives before the runs
  // list loads still resolves once it lands. Guard on the handled nonce: the
  // maps change identity on every refetch, and without this a still-pending
  // request would re-select on each `subagent_event`, yanking the user out of
  // any nested drill-down.
  useEffect(() => {
    if (!selectRequest) return
    const { nonce, runId, childSessionId } = selectRequest
    if (handledNonceRef.current === nonce) return
    let run: SubagentRun | undefined
    if (runId) run = byId.get(runId)
    else if (childSessionId) run = byChildSessionId.get(childSessionId)

    if (run) {
      handledNonceRef.current = nonce
      selectRun(run)
      return
    }
    // A childSessionId with no run row AFTER the list has loaded = a non-sub-agent
    // session (e.g. a skill fork) — open its bare transcript. Before load, fall
    // through and wait so a real run isn't downgraded to a runId-less view.
    if (!runId && childSessionId && loaded) {
      handledNonceRef.current = nonce
      setStack([{ runId: "", sessionId: childSessionId, agentId: "" }])
      return
    }
    // runId not found yet (its row hasn't reached the snapshot), or not loaded:
    // leave the nonce UNCONSUMED and re-resolve when the next snapshot lands —
    // the run is real (a chip is only clickable once it has a runId) and will
    // appear. Consuming here would strand the panel on the list.
  }, [selectRequest, byId, byChildSessionId, loaded, selectRun])

  const pushNested = useCallback(
    (target: SubagentOpenTarget) => {
      const runId = target.runId
      const childSessionId = target.childSessionId
      if (!runId || !childSessionId) return
      setStack((s) =>
        s[s.length - 1]?.runId === runId
          ? s
          : [
              ...s,
              { runId, sessionId: childSessionId, agentId: target.agentId, label: target.label },
            ],
      )
      resetDisclosures()
    },
    [resetDisclosures],
  )

  /** Pop one level; from the root entry this returns to the list. */
  const goBack = useCallback(() => {
    setStack((s) => s.slice(0, -1))
    resetDisclosures()
  }, [resetDisclosures])

  const truncateStack = useCallback(
    (len: number) => {
      setStack((s) => (len >= s.length ? s : s.slice(0, len)))
      resetDisclosures()
    },
    [resetDisclosures],
  )

  // Reset when the host session CHANGES (contentKey remount is the primary
  // path; this is a defensive belt-and-braces). The ref guard is essential:
  // effects also fire on mount, and this one is declared after the selection
  // effects — without it, opening the panel from a chip / workspace row would
  // wipe the selection they just made, dropping the user back on the list.
  const lastSessionRef = useRef(sessionId)
  useEffect(() => {
    if (lastSessionRef.current === sessionId) return
    lastSessionRef.current = sessionId
    setStack([])
    handledNonceRef.current = null
  }, [sessionId])

  const handleCancel = useCallback(async () => {
    if (!selectedRunId || isTerminal || cancelling) return
    setCancelling(true)
    try {
      await getTransport().call<{ status?: string }>("cancel_runtime_task", {
        kind: "subagent",
        id: selectedRunId,
      })
    } catch {
      /* refetch/event will reconcile the real status */
    } finally {
      setCancelling(false)
    }
  }, [selectedRunId, isTerminal, cancelling])

  const topSessionId = top?.sessionId ?? null
  const handleChildMeta = useCallback(
    (meta: SessionMeta | null) => {
      if (topSessionId) setChildMeta({ sessionId: topSessionId, meta })
    },
    [topSessionId],
  )

  const selectedAgent = detail ? agentsMap.get(detail.childAgentId) : undefined
  const modelLabel = formatModelLabel(detail?.modelUsed)
  const durationLabel = useMemo(
    () => (detail?.durationMs != null ? formatDuration(detail.durationMs) : null),
    [detail?.durationMs],
  )

  const closeButton = (
    <button
      type="button"
      onClick={onClose}
      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
      aria-label={t("common.close", "Close")}
    >
      <X className="h-4 w-4" />
    </button>
  )

  // ── List view ────────────────────────────────────────────────────────
  if (!top) {
    return (
      <div className="flex h-full min-h-0 flex-col bg-background">
        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border/60 px-3">
          <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="text-sm font-medium">{t("subagentPanel.title", "Sub-agents")}</span>
          {closeButton}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {runs.length === 0 ? (
            <div className="px-2 py-8 text-center text-xs text-muted-foreground">
              {t("subagentPanel.empty", "No sub-agent runs in this session yet")}
            </div>
          ) : (
            <>
              {runningRuns.length > 0 && (
                <>
                  <GroupHeading>
                    {t("subagentPanel.runningCount", {
                      count: runningCount,
                      defaultValue: "{{count}} running",
                    })}
                  </GroupHeading>
                  {runningRuns.map((run) => (
                    <SubagentRunRow
                      key={run.runId}
                      run={run}
                      agent={agentsMap.get(run.childAgentId)}
                      onClick={() => selectRun(run)}
                    />
                  ))}
                </>
              )}
              {finishedRuns.length > 0 && (
                <>
                  <GroupHeading>
                    {t("subagentPanel.finishedCount", {
                      count: finishedCount,
                      defaultValue: "{{count}} finished",
                    })}
                  </GroupHeading>
                  {finishedRuns.map((run) => (
                    <SubagentRunRow
                      key={run.runId}
                      run={run}
                      agent={agentsMap.get(run.childAgentId)}
                      onClick={() => selectRun(run)}
                    />
                  ))}
                </>
              )}
            </>
          )}
        </div>
      </div>
    )
  }

  // ── Detail view ──────────────────────────────────────────────────────
  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border/60 px-2">
        <IconTip label={t("common.back", "Back")}>
          <button
            type="button"
            onClick={goBack}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
            aria-label={t("common.back", "Back")}
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
        </IconTip>
        <AgentAvatarBadge
          agent={selectedAgent ?? { id: top.agentId }}
          size="sm"
          colorSeed={top.agentId || top.sessionId}
        />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{entryTitle(top)}</span>
        {/* Only a real run has a status; a bare child-session entry (skill fork
            opened by session id, runId="") shows none rather than a misleading
            default "spawning". */}
        {selectedRunId && <SubagentStatusBadge status={status} />}
        {!isTerminal && selectedRunId && (
          <IconTip label={t("common.cancel", "Cancel")}>
            <button
              type="button"
              onClick={handleCancel}
              disabled={cancelling}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-red-500 disabled:opacity-50"
              aria-label={t("common.cancel", "Cancel")}
            >
              <XCircle className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
        <IconTip label={t("common.refresh", "Refresh")}>
          <button
            type="button"
            onClick={() => setReloadToken((n) => n + 1)}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
            aria-label={t("common.refresh", "Refresh")}
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        {closeButton}
      </div>

      {/* Breadcrumb — only for nested drill-downs, where "back" alone loses context. */}
      {stack.length > 1 && (
        <div className="flex h-7 shrink-0 items-center gap-1 px-2 text-xs">
          {stack.map((entry, idx) => (
            <div key={`${entry.runId}-${idx}`} className="flex min-w-0 items-center gap-1">
              {idx > 0 && <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />}
              <button
                type="button"
                onClick={() => truncateStack(idx + 1)}
                className={cn(
                  "max-w-[140px] truncate rounded px-1 py-0.5 transition-colors hover:bg-secondary/60",
                  idx === stack.length - 1
                    ? "font-medium text-foreground"
                    : "text-muted-foreground",
                )}
              >
                {entryTitle(entry)}
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Task + quiet meta line */}
      {(detail?.task || durationLabel || modelLabel || detail?.inputTokens != null) && (
        <div className="shrink-0 px-3 py-2">
          {detail?.task && (
            <p className="line-clamp-2 text-xs leading-relaxed text-muted-foreground">
              {detail.task}
            </p>
          )}
          {(durationLabel || modelLabel || detail?.inputTokens != null) && (
            <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground">
              {durationLabel && <span className="tabular-nums">{durationLabel}</span>}
              {detail?.inputTokens != null && detail?.outputTokens != null && (
                <span className="tabular-nums">
                  {detail.inputTokens.toLocaleString()}↑ {detail.outputTokens.toLocaleString()}↓
                </span>
              )}
              {modelLabel && <span className="min-w-0 truncate">{modelLabel}</span>}
            </div>
          )}
        </div>
      )}

      {/* Pane switcher — result / details / conversation are three views of the
          same run, so they're mutually exclusive tabs rather than stacked
          disclosures competing for height with the transcript. */}
      <div className="flex shrink-0 items-center gap-2 px-3 pb-1.5">
        <div className="inline-flex h-7 items-center rounded-lg bg-muted p-0.5 text-muted-foreground">
          {detail?.result && (
            <PaneTab
              active={pane === "result"}
              label={t("subagentPanel.resultSection", "Result")}
              onSelect={() => setPaneOverride("result")}
            />
          )}
          <PaneTab
            active={pane === "conversation"}
            label={t("subagentPanel.conversationTab", "Conversation")}
            onSelect={() => setPaneOverride("conversation")}
          />
          {detail && (
            <PaneTab
              active={pane === "details"}
              label={t("subagentPanel.detailsSection", "Details")}
              onSelect={() => setPaneOverride("details")}
            />
          )}
        </div>
        {detail?.error && (
          <span className="ml-auto shrink-0 text-[11px] text-red-500">
            {t("executionStatus.subagent.status.error", "Error")}
          </span>
        )}
      </div>

      {/* One rounded card carries whichever pane is active — mirrors the
          workspace section cards and replaces a stack of divider lines. */}
      <div className="flex min-h-0 flex-1 flex-col px-3 pb-3">
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-border/80 bg-surface-floating shadow-sm">
          {pane === "result" && detail?.result && (
            <div
              className={cn(
                "min-h-0 flex-1 overflow-y-auto px-3 py-2.5 text-[12px] leading-relaxed",
                PANEL_SCROLL_FADE,
              )}
            >
              <MarkdownRenderer content={detail.result} />
            </div>
          )}

          {pane === "details" && detail && (
            <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-3 py-2.5">
              <SubagentRunDetails
                run={detail}
                sessionMeta={childMeta?.sessionId === top.sessionId ? childMeta.meta : null}
              />
              {detail.error && (
                <pre className="max-w-full whitespace-pre-wrap break-words rounded-lg bg-secondary/40 p-2 text-[11px] leading-relaxed text-red-400">
                  {detail.error}
                </pre>
              )}
            </div>
          )}

          {/* Kept mounted across pane switches (hidden, not unmounted) so the
              loaded transcript and its live stream survive — remounting refetches. */}
          <SubagentSessionView
            key={top.sessionId}
            sessionId={top.sessionId}
            agents={agents}
            className={cn("min-h-0 flex-1", pane !== "conversation" && "hidden")}
            reloadToken={reloadToken}
            onOpenSubagentRun={pushNested}
            onMeta={handleChildMeta}
          />
        </div>
      </div>
    </div>
  )
}
