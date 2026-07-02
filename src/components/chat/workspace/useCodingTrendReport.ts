import { useCallback, useEffect, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type {
  ApplyCodingImprovementProposalResult,
  CodingImprovementActionPlan,
  CodingImprovementPromotionPlan,
  CodingImprovementProposal,
  CodingTrendReport,
  GenerateCodingImprovementProposalsResult,
  PromoteCodingImprovementProposalResult,
} from "@/lib/transport"

export interface CodingTrendReportState {
  report: CodingTrendReport | null
  loading: boolean
  generating: boolean
  updatingProposalId: string | null
  previewingProposalId: string | null
  applyingProposalId: string | null
  previewingPromotionId: string | null
  promotingProposalId: string | null
  actionPlan: CodingImprovementActionPlan | null
  promotionPlan: CodingImprovementPromotionPlan | null
  error: string | null
  refresh: () => void
  generateProposals: () => Promise<GenerateCodingImprovementProposalsResult | null>
  updateProposalStatus: (
    proposalId: string,
    status: "rejected" | "draft",
  ) => Promise<CodingImprovementProposal | null>
  previewProposalAction: (proposalId: string) => Promise<CodingImprovementActionPlan | null>
  applyProposal: (proposalId: string) => Promise<ApplyCodingImprovementProposalResult | null>
  previewProposalPromotion: (proposalId: string) => Promise<CodingImprovementPromotionPlan | null>
  promoteProposal: (proposalId: string) => Promise<PromoteCodingImprovementProposalResult | null>
}

const CODING_TREND_WINDOW_DAYS = 30
const CODING_TREND_EVENT_REFRESH_DEBOUNCE_MS = 600

function payloadBelongsToSession(payload: unknown, sessionId: string): boolean {
  if (typeof payload !== "object" || payload === null) return true
  const value = (payload as { sessionId?: unknown }).sessionId
  return typeof value !== "string" || value === sessionId
}

export function useCodingTrendReport(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): CodingTrendReportState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [report, setReport] = useState<CodingTrendReport | null>(null)
  const [loading, setLoading] = useState(false)
  const [generating, setGenerating] = useState(false)
  const [updatingProposalId, setUpdatingProposalId] = useState<string | null>(null)
  const [previewingProposalId, setPreviewingProposalId] = useState<string | null>(null)
  const [applyingProposalId, setApplyingProposalId] = useState<string | null>(null)
  const [previewingPromotionId, setPreviewingPromotionId] = useState<string | null>(null)
  const [promotingProposalId, setPromotingProposalId] = useState<string | null>(null)
  const [actionPlan, setActionPlan] = useState<CodingImprovementActionPlan | null>(null)
  const [promotionPlan, setPromotionPlan] = useState<CodingImprovementPromotionPlan | null>(null)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchReport = useCallback(() => {
    if (disabled || !sessionId || incognito) {
      reqRef.current += 1
      setReport(null)
      setActionPlan(null)
      setPromotionPlan(null)
      setLoading(false)
      setError(null)
      return
    }
    const req = ++reqRef.current
    setLoading(true)
    setError(null)
    getTransport()
      .call<CodingTrendReport>("get_coding_trend_report", {
        sessionId,
        windowDays: CODING_TREND_WINDOW_DAYS,
      })
      .then((next) => {
        if (reqRef.current !== req) return
        setReport(next)
        setLoading(false)
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to load coding trend report", e)
        setError(message)
        setLoading(false)
      })
  }, [disabled, incognito, sessionId])

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchReport()
    })
    return () => {
      cancelled = true
    }
  }, [fetchReport])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) fetchReport()
      })
    }
    return () => {
      cancelled = true
    }
  }, [fetchReport, turnActive])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const transport = getTransport()
    const scheduleRefresh = (payload?: unknown) => {
      if (payload !== undefined && !payloadBelongsToSession(payload, sessionId)) return
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchReport()
      }, CODING_TREND_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const unsubs = [
      transport.listen("goal:created", scheduleRefresh),
      transport.listen("goal:updated", scheduleRefresh),
      transport.listen("goal:event", scheduleRefresh),
      transport.listen("workflow:created", scheduleRefresh),
      transport.listen("workflow:updated", scheduleRefresh),
      transport.listen("workflow:event", scheduleRefresh),
      transport.listen("review:created", scheduleRefresh),
      transport.listen("review:updated", scheduleRefresh),
      transport.listen("review:finding_updated", scheduleRefresh),
      transport.listen("verification:created", scheduleRefresh),
      transport.listen("verification:updated", scheduleRefresh),
      transport.listen("verification:step_updated", scheduleRefresh),
      transport.listen("_lagged", () => scheduleRefresh()),
    ]
    return () => {
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
      unsubs.forEach((unsub) => unsub())
    }
  }, [disabled, fetchReport, incognito, sessionId])

  const generateProposals = useCallback(async () => {
    if (!sessionId || disabled || incognito) return null
    setGenerating(true)
    setError(null)
    try {
      const result = await getTransport().call<GenerateCodingImprovementProposalsResult>(
        "generate_coding_improvement_proposals",
        {
          sessionId,
          windowDays: CODING_TREND_WINDOW_DAYS,
        },
      )
      fetchReport()
      return result
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useCodingTrendReport", "Failed to generate improvement proposals", e)
      setError(message)
      return null
    } finally {
      setGenerating(false)
    }
  }, [disabled, fetchReport, incognito, sessionId])

  const updateProposalStatus = useCallback(
    async (proposalId: string, status: "rejected" | "draft") => {
      if (!sessionId || disabled || incognito) return null
      setUpdatingProposalId(proposalId)
      setError(null)
      try {
        const proposal = await getTransport().call<CodingImprovementProposal>(
          "update_coding_improvement_proposal_status",
          { proposalId, status },
        )
        fetchReport()
        return proposal
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to update proposal status", e)
        setError(message)
        return null
      } finally {
        setUpdatingProposalId(null)
      }
    },
    [disabled, fetchReport, incognito, sessionId],
  )

  const previewProposalAction = useCallback(
    async (proposalId: string) => {
      if (!sessionId || disabled || incognito) return null
      setPreviewingProposalId(proposalId)
      setError(null)
      try {
        const plan = await getTransport().call<CodingImprovementActionPlan>(
          "preview_coding_improvement_proposal_action",
          { proposalId },
        )
        setActionPlan(plan)
        return plan
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to preview improvement action", e)
        setError(message)
        return null
      } finally {
        setPreviewingProposalId(null)
      }
    },
    [disabled, incognito, sessionId],
  )

  const applyProposal = useCallback(
    async (proposalId: string) => {
      if (!sessionId || disabled || incognito) return null
      setApplyingProposalId(proposalId)
      setError(null)
      try {
        const result = await getTransport().call<ApplyCodingImprovementProposalResult>(
          "apply_coding_improvement_proposal",
          { proposalId },
        )
        setActionPlan(result.plan)
        fetchReport()
        return result
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to apply improvement proposal", e)
        setError(message)
        return null
      } finally {
        setApplyingProposalId(null)
      }
    },
    [disabled, fetchReport, incognito, sessionId],
  )

  const previewProposalPromotion = useCallback(
    async (proposalId: string) => {
      if (!sessionId || disabled || incognito) return null
      setPreviewingPromotionId(proposalId)
      setError(null)
      try {
        const plan = await getTransport().call<CodingImprovementPromotionPlan>(
          "preview_coding_improvement_proposal_promotion",
          { proposalId },
        )
        setPromotionPlan(plan)
        return plan
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to preview improvement promotion", e)
        setError(message)
        return null
      } finally {
        setPreviewingPromotionId(null)
      }
    },
    [disabled, incognito, sessionId],
  )

  const promoteProposal = useCallback(
    async (proposalId: string) => {
      if (!sessionId || disabled || incognito) return null
      setPromotingProposalId(proposalId)
      setError(null)
      try {
        const result = await getTransport().call<PromoteCodingImprovementProposalResult>(
          "promote_coding_improvement_proposal",
          { proposalId },
        )
        setPromotionPlan(result.plan)
        fetchReport()
        return result
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useCodingTrendReport", "Failed to promote improvement proposal", e)
        setError(message)
        return null
      } finally {
        setPromotingProposalId(null)
      }
    },
    [disabled, fetchReport, incognito, sessionId],
  )

  return {
    report,
    loading,
    generating,
    updatingProposalId,
    previewingProposalId,
    applyingProposalId,
    previewingPromotionId,
    promotingProposalId,
    actionPlan,
    promotionPlan,
    error,
    refresh: fetchReport,
    generateProposals,
    updateProposalStatus,
    previewProposalAction,
    applyProposal,
    previewProposalPromotion,
    promoteProposal,
  }
}
