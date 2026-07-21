import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ExternalLink, GitPullRequest, Loader2, RefreshCw, X } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type {
  GitMutationResult,
  GitPullRequestFeedback,
  SessionGitControlSnapshot,
} from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "@/lib/openExternalUrl"
import {
  PullRequestDetailsContent,
} from "./GitControlCard"
import {
  buildChecksFixPrompt,
  buildCommentsFixPrompt,
  buildMergeConflictFixPrompt,
  buildPullRequestFixPrompt,
  hasPullRequestConflicts,
  isActionableReview,
  pullRequestUnavailableReason,
} from "./gitPullRequestUtils"

interface PullRequestPanelProps {
  sessionId: string
  expectedUrl?: string | null
  onClose: () => void
  onFillInput?: (value: string) => void
}

export function PullRequestPanel({
  sessionId,
  expectedUrl,
  onClose,
  onFillInput,
}: PullRequestPanelProps) {
  const { t } = useTranslation()
  const [feedback, setFeedback] = useState<GitPullRequestFeedback | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [autoMergeOpen, setAutoMergeOpen] = useState(false)
  const [autoMergeMethod, setAutoMergeMethod] = useState<"merge" | "squash" | "rebase">("squash")
  const [autoMergeBusy, setAutoMergeBusy] = useState(false)
  const requestGenerationRef = useRef(0)
  const inFlightRequestRef = useRef<{
    generation: number
    promise: Promise<GitPullRequestFeedback | null>
  } | null>(null)
  const currentPullRequestNumber = feedback?.preflight.current?.number ?? null

  const loadFeedback = useCallback(() => {
    const inFlight = inFlightRequestRef.current
    if (inFlight) return inFlight.promise
    const generation = ++requestGenerationRef.current
    const rawPromise = getTransport().call<GitPullRequestFeedback>(
      "load_session_git_pr_feedback_cmd",
      { sessionId },
    )
    setLoading(true)
    const promise = rawPromise
      .then((next) => {
        if (requestGenerationRef.current !== generation) return null
        setFeedback(next)
        setError(null)
        return next
      })
      .catch((cause) => {
        if (requestGenerationRef.current !== generation) return null
        setError(cause instanceof Error ? cause.message : String(cause))
        return null
      })
      .finally(() => {
        if (inFlightRequestRef.current?.generation === generation) {
          inFlightRequestRef.current = null
        }
        if (requestGenerationRef.current === generation) setLoading(false)
      })
    inFlightRequestRef.current = { generation, promise }
    return promise
  }, [sessionId])

  useEffect(() => {
    setFeedback(null)
    setError(null)
    setAutoMergeOpen(false)
    void loadFeedback()
    return () => {
      requestGenerationRef.current += 1
      inFlightRequestRef.current = null
    }
  }, [loadFeedback])

  useEffect(() => {
    if (currentPullRequestNumber === null) return
    const timer = window.setInterval(() => void loadFeedback(), 30_000)
    return () => {
      window.clearInterval(timer)
    }
  }, [currentPullRequestNumber, loadFeedback])

  const fillPrompt = useCallback((prompt: string) => {
    if (!onFillInput) return
    onFillInput(prompt)
    toast.success(t("workspace.git.fixPromptReady", "修复要求已填入输入框，请确认后发送"))
  }, [onFillInput, t])

  const enableAutoMerge = async () => {
    if (autoMergeBusy) return
    setAutoMergeBusy(true)
    try {
      const snapshot = await getTransport().call<SessionGitControlSnapshot>(
        "load_session_git_control_cmd",
        { sessionId },
      )
      const result = await getTransport().call<GitMutationResult>(
        "enable_session_git_pr_auto_merge_cmd",
        {
          sessionId,
          input: {
            requestId: crypto.randomUUID(),
            expectedRevision: snapshot.revision,
            method: autoMergeMethod,
            confirmAutoMerge: true,
          },
        },
      )
      setAutoMergeOpen(false)
      toast.success(result.message)
      await loadFeedback()
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setAutoMergeBusy(false)
    }
  }

  const pullRequest = feedback?.preflight.current ?? null
  const failedChecks = feedback?.checks.filter(
    (check) => check.bucket === "fail" || check.bucket === "cancel",
  ) ?? []
  const comments = feedback?.reviewComments.filter(
    (comment) => !comment.isResolved && !comment.isOutdated,
  ) ?? []
  const reviews = (pullRequest?.reviews ?? []).filter(isActionableReview)
  const mergeConflicts = pullRequest ? hasPullRequestConflicts(pullRequest) : false
  const hasFixableFeedback = failedChecks.length > 0
    || comments.length > 0
    || reviews.length > 0
    || mergeConflicts
  const unavailableReason = feedback && !feedback.preflight.available
    ? pullRequestUnavailableReason(t, feedback.preflight)
    : null
  const emptyMessage = loading
    ? expectedUrl
      ? t("workspace.git.prCreatedSyncing", "拉取请求已创建，正在同步详情")
      : t("workspace.git.findingPullRequest", "查找关联拉取请求")
    : expectedUrl
      ? t("workspace.git.prCreatedSyncFailed", "拉取请求已创建，但详情尚未同步")
      : unavailableReason ?? (error
        ? t("workspace.git.prFeedbackUnavailable", "PR 检查与评论不可用")
        : t("workspace.git.prNotFound", "当前分支尚未关联拉取请求"))

  return (
    <>
      {pullRequest ? (
        <PullRequestDetailsContent
          pullRequest={pullRequest}
          feedback={feedback}
          loading={loading}
          refreshError={error}
          onClose={onClose}
          onRefresh={() => void loadFeedback()}
          onFixAll={!error && onFillInput && hasFixableFeedback
            ? () => fillPrompt(buildPullRequestFixPrompt(
                pullRequest,
                failedChecks,
                comments,
                reviews,
                mergeConflicts,
              ))
            : undefined}
          onFixChecks={!error && onFillInput && failedChecks.length > 0
            ? (checks) => fillPrompt(buildChecksFixPrompt(pullRequest, checks))
            : undefined}
          onFixConflict={!error && onFillInput && mergeConflicts
            ? () => fillPrompt(buildMergeConflictFixPrompt(pullRequest))
            : undefined}
          onFixComments={!error && onFillInput && (comments.length > 0 || reviews.length > 0)
            ? (selectedComments, selectedReviews) => fillPrompt(
                buildCommentsFixPrompt(pullRequest, selectedComments, selectedReviews),
              )
            : undefined}
          onEnableAutoMerge={!error && !pullRequest.autoMergeEnabled && !mergeConflicts
            ? () => setAutoMergeOpen(true)
            : undefined}
        />
      ) : (
        <div className="flex h-full min-h-0 flex-col">
          <div className="flex items-center gap-2 border-b px-3 py-2">
            <GitPullRequest className="h-4 w-4 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate text-sm font-medium">
              {t("workspace.git.pullRequestPanelTitle", "拉取请求")}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={onClose}
              aria-label={t("common.close", "关闭")}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
          <div className="flex flex-1 items-center justify-center p-6 text-center text-sm text-muted-foreground">
            <div className="flex max-w-sm flex-col items-center gap-3">
              <span className="inline-flex items-center gap-2">
                {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {emptyMessage}
              </span>
              {!loading && error ? (
                <span className="max-w-full break-words text-xs text-destructive">{error}</span>
              ) : null}
              {!loading ? (
                <div className="flex flex-wrap items-center justify-center gap-2">
                  <Button type="button" size="sm" variant="outline" onClick={() => void loadFeedback()}>
                    <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                    {t("common.retry", "重试")}
                  </Button>
                  {expectedUrl ? (
                    <Button type="button" size="sm" variant="outline" onClick={() => openExternalUrl(expectedUrl)}>
                      <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                      {t("workspace.git.openPullRequest", "在 GitHub 打开")}
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      )}

      <Dialog open={autoMergeOpen} onOpenChange={setAutoMergeOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("workspace.git.enableAutoMerge", "启用自动合并")}</DialogTitle>
            <DialogDescription>
              {t(
                "workspace.git.autoMergeWarning",
                "当分支保护条件满足时，GitHub 可能立即合并此拉取请求。该操作需要明确确认。",
              )}
            </DialogDescription>
          </DialogHeader>
          <label className="space-y-1.5 text-sm">
            <span className="font-medium">{t("workspace.git.mergeMethod", "合并方式")}</span>
            <select
              className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
              value={autoMergeMethod}
              onChange={(event) => setAutoMergeMethod(
                event.target.value as "merge" | "squash" | "rebase",
              )}
            >
              <option value="squash">{t("workspace.git.mergeSquash", "压缩合并")}</option>
              <option value="merge">{t("workspace.git.mergeCommit", "创建合并提交")}</option>
              <option value="rebase">{t("workspace.git.mergeRebase", "变基合并")}</option>
            </select>
          </label>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAutoMergeOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void enableAutoMerge()} disabled={autoMergeBusy}>
              {autoMergeBusy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("workspace.git.confirmAutoMerge", "确认启用")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
