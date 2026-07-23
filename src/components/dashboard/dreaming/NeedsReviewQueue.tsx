import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { ClipboardCheck, Loader2 } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import ClaimReviewActions, { type ReviewableClaim } from "./ClaimReviewActions"
import {
  claimReviewActionErrorToast,
  type ClaimReviewActionErrorToast,
} from "./claimReviewActionFeedback"

// Mirrors the action fields of ha-core `ClaimRecord` plus the few display
// columns the queue renders.
interface ClaimRecord extends ReviewableClaim {
  claimType: string
  confidence: number
}

/**
 * The Lucid Review queue (design §5.1): claims the pipeline flagged
 * `needs_review` (low-confidence, conflicts, uncertain scope). Each row expands
 * to the full correction toolbar so the user can approve / edit / reject /
 * move-scope / forget without leaving the Dashboard. Refreshes on
 * `memory:claim_changed` / `memory:review_required` so it stays live as the
 * pipeline runs.
 */
export default function NeedsReviewQueue() {
  const { t } = useTranslation()
  const [claims, setClaims] = useState<ClaimRecord[]>([])
  const [loading, setLoading] = useState(false)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [loadError, setLoadError] = useState<ClaimReviewActionErrorToast | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const list = await getTransport().call<ClaimRecord[]>("claim_list", {
        status: "needs_review",
        limit: 100,
      })
      setClaims(list ?? [])
      setLoadError(null)
      // Drop a stale expansion: a claim acted on usually leaves needs_review,
      // so its id is no longer in the list.
      setExpandedId((prev) => (prev && (list ?? []).some((c) => c.id === prev) ? prev : null))
    } catch (e) {
      logger.error("dashboard", "NeedsReviewQueue::list", "Failed to list review claims", e)
      setLoadError(claimReviewActionErrorToast("loadQueue", t, e))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void load()
    const t = getTransport()
    const offChanged = t.listen("memory:claim_changed", () => void load())
    const offReview = t.listen("memory:review_required", () => void load())
    return () => {
      offChanged()
      offReview()
    }
  }, [load])

  return (
    <div className="border border-border/60 rounded-lg overflow-hidden">
      <div className="px-3 py-2 border-b border-border/60 bg-secondary/20 text-xs font-medium flex items-center gap-2">
        <ClipboardCheck className="h-3.5 w-3.5 text-sky-500" />
        {t("dashboard.dreaming.review.queueTitle")} ({claims.length})
        {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
      </div>
      {loadError && (
        <div className="border-b border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="font-medium text-foreground">{loadError.title}</div>
          {loadError.description && (
            <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
          )}
        </div>
      )}
      <div className="max-h-[320px] overflow-y-auto">
        {claims.length === 0 ? (
          <div className="px-3 py-6 text-xs text-muted-foreground text-center">
            {t("dashboard.dreaming.review.queueEmpty")}
          </div>
        ) : (
          claims.map((c) => {
            const expanded = expandedId === c.id
            const scopeName =
              c.scopeType === "global"
                ? t("settings.memoryScopeGlobal")
                : c.scopeType === "agent"
                  ? t("settings.memoryScopeAgent")
                  : c.scopeType === "project"
                    ? t("settings.memoryScopeProject")
                    : c.scopeType === "session"
                      ? t("dashboard.columns.session")
                      : c.scopeType
            const scopeLabel =
              c.scopeType === "global" ? scopeName : `${scopeName}:${c.scopeId ?? "?"}`
            return (
              <div key={c.id} className="border-b border-border/30 last:border-0">
                <button
                  onClick={() => setExpandedId(expanded ? null : c.id)}
                  className={`w-full text-left px-3 py-2 text-xs hover:bg-secondary/40 transition-colors ${
                    expanded ? "bg-secondary" : ""
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="h-2 w-2 rounded-full shrink-0 bg-sky-500" />
                    <span className="truncate">{c.content}</span>
                  </div>
                  <div className="text-[10px] text-muted-foreground mt-0.5 font-mono">
                    {t(`settings.claimType_${c.claimType}`, c.claimType)} ·{" "}
                    {scopeLabel} ·{" "}
                    {(c.confidence * 100).toFixed(0)}%
                  </div>
                </button>
                {expanded && (
                  <div className="px-3 pb-3 pt-1">
                    <ClaimReviewActions claim={c} onChanged={load} />
                  </div>
                )}
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}
