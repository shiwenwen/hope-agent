import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  Check,
  CheckCircle2,
  ChevronDown,
  Circle,
  CircleX,
  Clock3,
  ExternalLink,
  GitBranch,
  GitCommitHorizontal,
  GitCompare,
  GitMerge,
  GitPullRequest,
  HardDrive,
  Loader2,
  MessageCircle,
  Plus,
  RefreshCw,
  Search,
  Trees,
  UserRound,
  WandSparkles,
  X,
  type LucideIcon,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { openExternalUrl } from "@/lib/openExternalUrl"
import type {
  GitMutationResult,
  GitPullRequestCheck,
  GitPullRequestFeedback,
  GitPullRequestInfo,
  GitPullRequestPreflight,
  GitPullRequestReview,
  GitPullRequestReviewComment,
  ManagedWorktree,
  SessionGitDiffSnapshot,
} from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { FileDeltaCounter } from "@/components/chat/message/FileDeltaCounter"
import type { SessionGitControlState } from "./useSessionGitControl"
import {
  buildChecksFixPrompt,
  buildCommentsFixPrompt,
  buildMergeConflictFixPrompt,
  hasPullRequestConflicts,
  isActionableReview,
  pullRequestUnavailableReason,
} from "./gitPullRequestUtils"

interface GitControlCardProps {
  sessionId: string
  state: SessionGitControlState
  managedWorktrees: ManagedWorktree[]
  onOpenGitDiff: (
    snapshot: SessionGitDiffSnapshot,
    sessionId: string,
    reviewComments?: GitPullRequestReviewComment[],
  ) => void
  onFillInput?: (value: string) => void
  onOpenPullRequest?: (expectedUrl?: string | null) => void
  managedWorktreeControls?: ReactNode
}

interface PrFeedbackViewState {
  key: string | null
  feedback: GitPullRequestFeedback | null
  error: string | null
}

interface PrFeedbackRequest {
  key: string
  token: symbol
  promise: Promise<GitPullRequestFeedback | null>
}

export function GitControlCard({
  sessionId,
  state,
  managedWorktrees,
  onOpenGitDiff,
  onFillInput,
  onOpenPullRequest,
  managedWorktreeControls,
}: GitControlCardProps) {
  const { t } = useTranslation()
  const snapshot = state.snapshot
  const hasGithubRemote = Boolean(snapshot?.remotes.some((remote) => remote.isGithub))
  const prContextKey = snapshot && !snapshot.detached && snapshot.branch
    ? `${sessionId}:${snapshot.head ?? ""}:${snapshot.branch}`
    : null
  const canLoadPrFeedback = Boolean(
    prContextKey && hasGithubRemote && snapshot?.sync.upstream,
  )
  const prFeedbackKey = canLoadPrFeedback && snapshot
    ? `${prContextKey}:${snapshot.sync.upstream}`
    : null
  const [action, setAction] = useState<string | null>(null)
  const [branchOpen, setBranchOpen] = useState(false)
  const [branchQuery, setBranchQuery] = useState("")
  const [newBranch, setNewBranch] = useState("")
  const [commitOpen, setCommitOpen] = useState(false)
  const [commitSubject, setCommitSubject] = useState("")
  const [commitBody, setCommitBody] = useState("")
  const [stageAll, setStageAll] = useState(false)
  const [pushAfter, setPushAfter] = useState(false)
  const [prOpen, setPrOpen] = useState(false)
  const [prTitle, setPrTitle] = useState("")
  const [prBody, setPrBody] = useState("")
  const [prBase, setPrBase] = useState("")
  const [prDraft, setPrDraft] = useState(true)
  const [prPushFirst, setPrPushFirst] = useState(true)
  const [prPushRequired, setPrPushRequired] = useState(false)
  const [prPreflight, setPrPreflight] = useState<GitPullRequestPreflight | null>(null)
  const [prPreflightKey, setPrPreflightKey] = useState<string | null>(null)
  const [prFeedbackView, setPrFeedbackView] = useState<PrFeedbackViewState>({
    key: null,
    feedback: null,
    error: null,
  })
  const [prFeedbackLoadingKey, setPrFeedbackLoadingKey] = useState<string | null>(null)
  const prFeedbackRequestsRef = useRef<Map<string, PrFeedbackRequest>>(new Map())
  const currentPrFeedbackKeyRef = useRef<string | null>(prFeedbackKey)
  currentPrFeedbackKeyRef.current = prFeedbackKey
  const prFeedback = prFeedbackView.key === prFeedbackKey ? prFeedbackView.feedback : null
  const prFeedbackError = prFeedbackView.key === prFeedbackKey ? prFeedbackView.error : null
  const prFeedbackLoading = prFeedbackKey !== null && prFeedbackLoadingKey === prFeedbackKey
  const visiblePrPreflight = prContextKey !== null && prPreflightKey === prContextKey
    ? prPreflight
    : null

  const branches = useMemo(() => {
    const query = branchQuery.trim().toLowerCase()
    if (!snapshot) return []
    return query
      ? snapshot.branches.filter((branch) => branch.name.toLowerCase().includes(query))
      : snapshot.branches
  }, [branchQuery, snapshot])
  const branchGroups = useMemo(
    () => [
      {
        kind: "local" as const,
        label: t("workspace.git.localBranches", "本地分支"),
        branches: branches.filter((branch) => branch.kind === "local"),
      },
      {
        kind: "remote" as const,
        label: t("workspace.git.remoteBranches", "远端跟踪分支"),
        branches: branches.filter((branch) => branch.kind === "remote"),
      },
    ],
    [branches, t],
  )

  const loadPrFeedback = useCallback(() => {
    const requestKey = prFeedbackKey
    if (!requestKey) return Promise.resolve(null)
    const inFlight = prFeedbackRequestsRef.current.get(requestKey)
    if (inFlight) return inFlight.promise

    const token = Symbol(requestKey)
    const rawPromise = getTransport().call<GitPullRequestFeedback>(
      "load_session_git_pr_feedback_cmd",
      { sessionId },
    )
    setPrFeedbackLoadingKey(requestKey)
    const promise = rawPromise
      .then((feedback) => {
        if (
          currentPrFeedbackKeyRef.current !== requestKey
          || prFeedbackRequestsRef.current.get(requestKey)?.token !== token
        ) return null
        setPrFeedbackView({ key: requestKey, feedback, error: null })
        setPrPreflight(feedback.preflight)
        setPrPreflightKey(prContextKey)
        return feedback
      })
      .catch((error) => {
        if (
          currentPrFeedbackKeyRef.current !== requestKey
          || prFeedbackRequestsRef.current.get(requestKey)?.token !== token
        ) return null
        const message = error instanceof Error ? error.message : String(error)
        setPrFeedbackView((current) => ({
          key: requestKey,
          feedback: current.key === requestKey ? current.feedback : null,
          error: message,
        }))
        return null
      })
      .finally(() => {
        if (prFeedbackRequestsRef.current.get(requestKey)?.token === token) {
          prFeedbackRequestsRef.current.delete(requestKey)
        }
        if (currentPrFeedbackKeyRef.current === requestKey) {
          setPrFeedbackLoadingKey((current) => current === requestKey ? null : current)
        }
      })
    prFeedbackRequestsRef.current.set(requestKey, { key: requestKey, token, promise })
    return promise
  }, [prContextKey, prFeedbackKey, sessionId])

  useEffect(() => {
    setPrFeedbackView({ key: prFeedbackKey, feedback: null, error: null })
    setPrPreflight(null)
    setPrPreflightKey(prContextKey)
    if (!prFeedbackKey) {
      setPrFeedbackLoadingKey(null)
      return
    }
    void loadPrFeedback()
  }, [loadPrFeedback, prContextKey, prFeedbackKey])

  const currentPrNumber = prFeedback?.preflight.current?.number ?? null
  useEffect(() => {
    if (!prFeedbackKey || currentPrNumber === null) return
    const timer = window.setInterval(() => void loadPrFeedback(), 30_000)
    return () => {
      window.clearInterval(timer)
    }
  }, [currentPrNumber, loadPrFeedback, prFeedbackKey])

  const fillFixPrompt = useCallback(
    (value: string) => {
      if (!onFillInput) return
      onFillInput(value)
      toast.success(t("workspace.git.fixPromptReady", "修复要求已填入输入框，请确认后发送"))
    },
    [onFillInput, t],
  )

  const run = async <T,>(key: string, task: () => Promise<T>): Promise<T | null> => {
    if (action) return null
    setAction(key)
    try {
      const value = await task()
      state.refresh()
      return value
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
      return null
    } finally {
      setAction(null)
    }
  }

  const openChanges = () =>
    void run("diff", async () => {
      const scope = snapshot?.dirty.unstagedFiles || snapshot?.dirty.untrackedFiles
        ? "unstaged"
        : "staged"
      const diff = await getTransport().call<SessionGitDiffSnapshot>(
        "load_session_git_diff_snapshot_cmd",
        { sessionId, scope },
      )
      onOpenGitDiff(
        diff,
        sessionId,
        prFeedback?.reviewComments.filter(
          (comment) => !comment.isResolved && !comment.isOutdated,
        ) ?? [],
      )
      return diff
    })

  const switchBranch = (fullRef: string) =>
    void run("branch", async () => {
      if (!snapshot) throw new Error("Git snapshot unavailable")
      const result = await getTransport().call<GitMutationResult>(
        "switch_session_git_branch_cmd",
        {
          sessionId,
          input: {
            requestId: crypto.randomUUID(),
            expectedRevision: snapshot.revision,
            fullRef,
          },
        },
      )
      setBranchOpen(false)
      toast.success(t("workspace.git.branchSwitched", "分支已切换"))
      return result
    })

  const createBranch = () =>
    void run("branch", async () => {
      if (!snapshot || !newBranch.trim()) throw new Error("请输入分支名")
      const result = await getTransport().call<GitMutationResult>(
        "create_session_git_branch_cmd",
        {
          sessionId,
          input: {
            requestId: crypto.randomUUID(),
            expectedRevision: snapshot.revision,
            name: newBranch.trim(),
          },
        },
      )
      setNewBranch("")
      setBranchOpen(false)
      toast.success(t("workspace.git.branchCreated", "分支已创建"))
      return result
    })

  const handoff = (target: "local" | "worktree", worktreeId?: string) =>
    void run("handoff", async () => {
      if (!snapshot) throw new Error("Git snapshot unavailable")
      const result = await getTransport().call<GitMutationResult>("handoff_session_git_cmd", {
        sessionId,
        input: {
          requestId: crypto.randomUUID(),
          expectedRevision: snapshot.revision,
          target,
          worktreeId,
        },
      })
      toast.success(t("workspace.git.handoffDone", "运行位置已安全交接"))
      return result
    })

  const submitCommit = () =>
    void run("commit", async () => {
      if (!snapshot) throw new Error("Git snapshot unavailable")
      const result = await getTransport().call<GitMutationResult>("commit_session_git_cmd", {
        sessionId,
        input: {
          requestId: crypto.randomUUID(),
          expectedRevision: snapshot.revision,
          subject: commitSubject,
          body: commitBody || null,
          stageAll,
          pushAfter,
          remote: snapshot.remotes.find((remote) => remote.isDefault)?.name ?? null,
        },
      })
      setCommitOpen(false)
      setCommitSubject("")
      setCommitBody("")
      if (result.warning) toast.warning(`${result.message}: ${result.warning}`)
      else toast.success(result.message)
      return result
    })

  const push = () =>
    void run("push", async () => {
      if (!snapshot) throw new Error("Git snapshot unavailable")
      const result = await getTransport().call<GitMutationResult>("push_session_git_cmd", {
        sessionId,
        input: {
          requestId: crypto.randomUUID(),
          expectedRevision: snapshot.revision,
          remote: snapshot.remotes.find((remote) => remote.isDefault)?.name ?? null,
          setUpstream: true,
        },
      })
      toast.success(result.message)
      return result
    })

  const openPullRequest = () => {
    const existing = prFeedback?.preflight.current || visiblePrPreflight?.current
    if (existing) {
      if (onOpenPullRequest) onOpenPullRequest()
      else openExternalUrl(existing.url)
      return
    }
    void run("pr-preflight", async () => {
      const preflight = await getTransport().call<GitPullRequestPreflight>(
        "session_git_pr_preflight_cmd",
        { sessionId },
      )
      setPrPreflight(preflight)
      setPrPreflightKey(prContextKey)
      if (preflight.current) {
        void loadPrFeedback()
        if (onOpenPullRequest) onOpenPullRequest()
        else openExternalUrl(preflight.current.url)
        return preflight
      }
      if (!preflight.available) throw new Error(preflight.errorMessage || "Pull Request 不可用")
      setPrBase(preflight.defaultBranch || "main")
      setPrTitle(commitSubject || snapshot?.branch || "")
      const pushRequired = snapshot?.sync.state === "noUpstream"
      setPrPushRequired(pushRequired)
      setPrPushFirst(pushRequired || snapshot?.sync.state === "ahead")
      setPrOpen(true)
      return preflight
    })
  }

  const submitPullRequest = () =>
    void run("pr-create", async () => {
      if (!snapshot) throw new Error("Git snapshot unavailable")
      const result = await getTransport().call<GitMutationResult>("create_session_git_pr_cmd", {
        sessionId,
        input: {
          requestId: crypto.randomUUID(),
          expectedRevision: snapshot.revision,
          title: prTitle,
          body: prBody || null,
          baseBranch: prBase,
          draft: prDraft,
          pushFirst: prPushRequired || prPushFirst,
          remote: snapshot.remotes.find((remote) => remote.isDefault)?.name ?? null,
        },
      })
      setPrOpen(false)
      toast.success(result.message)
      if (onOpenPullRequest) onOpenPullRequest(result.url)
      else if (result.url) openExternalUrl(result.url)
      return result
    })

  if (state.loading && !snapshot) {
    return <div className="flex h-28 items-center justify-center"><Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /></div>
  }
  if (!snapshot) return null

  const dirty = snapshot.dirty.changedFiles > 0
  const ahead = snapshot.sync.ahead
  const busy = Boolean(action)
  const localOnly = !snapshot.detached && snapshot.sync.state === "noUpstream"
  const hasPushRemote = snapshot.remotes.some((remote) => remote.isDefault)
    || snapshot.remotes.length === 1
  const currentPr = prFeedback?.preflight.current ?? null
  const knownPr = currentPr ?? visiblePrPreflight?.current ?? null
  const preflightUnavailable = Boolean(visiblePrPreflight && !visiblePrPreflight.available)
  const prUnavailable = preflightUnavailable || Boolean(prFeedbackError)
  const shouldRetryPrDiscovery = prUnavailable && !knownPr
  const prUnavailableLabel = preflightUnavailable
    ? pullRequestUnavailableReason(t, visiblePrPreflight)
    : t("workspace.git.prFeedbackUnavailable", "PR 检查与评论不可用")
  const unresolvedReviewComments =
    prFeedback?.reviewComments.filter((comment) => !comment.isResolved && !comment.isOutdated) ?? []
  const failedChecks =
    prFeedback?.checks.filter((check) => check.bucket === "fail" || check.bucket === "cancel") ?? []
  const reviewSummaries = currentPr?.reviews ?? []
  const actionableReviews = reviewSummaries.filter(isActionableReview)
  const mergeConflicts = currentPr ? hasPullRequestConflicts(currentPr) : false

  return (
    <>
      <div className="overflow-hidden rounded-xl border border-border/65 bg-secondary/10 p-1">
        <GitRow
          icon={GitCompare}
          label={t("workspace.git.changes", "变更")}
          onClick={openChanges}
          disabled={!dirty || busy}
          value={dirty ? `${snapshot.dirty.changedFiles}` : t("workspace.git.clean", "无变更")}
          trailing={(snapshot.status.linesAdded > 0 || snapshot.status.linesRemoved > 0) ? <FileDeltaCounter linesAdded={snapshot.status.linesAdded} linesRemoved={snapshot.status.linesRemoved} /> : null}
          loading={action === "diff"}
        />

        <DropdownMenu>
          <DropdownMenuTrigger
            className="flex h-9 w-full items-center gap-2 rounded-lg px-2.5 text-left transition-colors hover:bg-secondary disabled:cursor-not-allowed disabled:opacity-45"
            disabled={busy || !snapshot.capabilities.canHandoff}
          >
            {snapshot.activeLocation === "local" ? (
              <HardDrive className="h-4 w-4 shrink-0" />
            ) : (
              <Trees className="h-4 w-4 shrink-0" />
            )}
            <span className="min-w-0 flex-1 truncate text-sm font-medium">
              {t("workspace.git.location", "运行位置")}
            </span>
            <span className="max-w-[45%] truncate text-xs text-muted-foreground">
              {snapshot.activeLocation === "local"
                ? t("workspace.git.local", "本地")
                : t("workspace.git.worktree", "工作树")}
            </span>
            {action === "handoff" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5" />
            )}
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-64">
            <DropdownMenuItem disabled={snapshot.activeLocation === "local"} onSelect={() => handoff("local")}>
              <HardDrive className="mr-2 h-4 w-4" />
              {t("workspace.git.localCheckout", "本地检出")}
              {snapshot.activeLocation === "local" ? <Check className="ml-auto h-4 w-4" /> : null}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            {managedWorktrees.filter((worktree) => worktree.pathExists && worktree.state !== "archived").map((worktree) => (
              <DropdownMenuItem key={worktree.id} disabled={snapshot.managedWorktreeId === worktree.id} onSelect={() => handoff("worktree", worktree.id)}>
                <Trees className="mr-2 h-4 w-4" />
                <span className="truncate">{worktree.label || worktree.id}</span>
                {snapshot.managedWorktreeId === worktree.id ? <Check className="ml-auto h-4 w-4" /> : null}
              </DropdownMenuItem>
            ))}
            <DropdownMenuItem onSelect={() => handoff("worktree")}>
              <Plus className="mr-2 h-4 w-4" />
              {t("workspace.git.newWorktree", "新建工作树")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        {managedWorktreeControls}

        <GitRow
          icon={GitBranch}
          label={snapshot.detached ? t("workspace.git.createBranch", "创建分支") : t("workspace.git.branch", "分支")}
          value={snapshot.branch || `HEAD ${snapshot.head?.slice(0, 8) || ""}`}
          trailing={<ChevronDown className="h-3.5 w-3.5" />}
          onClick={() => setBranchOpen(true)}
          disabled={busy || !snapshot.capabilities.canCreateBranch}
          loading={action === "branch"}
        />

        <GitRow
          icon={GitCommitHorizontal}
          label={dirty
            ? t("workspace.git.commit", "提交")
            : localOnly
              ? t("workspace.git.pushBranch", "推送分支")
              : t("workspace.git.push", "推送")}
          value={!dirty && ahead > 0 ? t("workspace.git.aheadCount", "{{count}} 个提交", { count: ahead }) : undefined}
          onClick={() => dirty ? setCommitOpen(true) : push()}
          disabled={busy || snapshot.detached || (!dirty && (
            !snapshot.capabilities.canPush || (localOnly ? !hasPushRemote : ahead === 0)
          ))}
          loading={action === "commit" || action === "push"}
        />

        <GitRow
          icon={GitPullRequest}
          label={prFeedbackLoading && !knownPr
            ? t("workspace.git.findingPullRequest", "查找关联拉取请求")
            : knownPr
              ? t("workspace.git.viewPr", "查看拉取请求")
              : prUnavailable
                ? prUnavailableLabel
                : localOnly && hasGithubRemote
                  ? t("workspace.git.pushAndCreatePr", "推送并创建拉取请求")
                  : t("workspace.git.createPr", "创建拉取请求")}
          value={snapshot.detached
            ? t("workspace.git.createBranchFirst", "请先创建或切换分支")
            : !hasGithubRemote
              ? t("workspace.git.githubRemoteRequired", "需要 GitHub 远端")
              : shouldRetryPrDiscovery
                ? t("common.retry", "重试")
                : undefined}
          onClick={shouldRetryPrDiscovery
            ? () => void (prFeedbackKey ? loadPrFeedback() : openPullRequest())
            : openPullRequest}
          disabled={busy || snapshot.detached || !hasGithubRemote || prFeedbackLoading
            || (!knownPr && !snapshot.capabilities.canCreatePullRequest)}
          loading={action === "pr-preflight" || action === "pr-create"
            || (prFeedbackLoading && !knownPr)}
        />

        {currentPr && prFeedback ? (
          <>
            {prFeedbackError ? (
              <GitRow
                icon={CircleX}
                label={t("workspace.git.prFeedbackStale", "PR 状态刷新失败，当前数据可能已过期")}
                value={t("common.retry", "重试")}
                onClick={() => void loadPrFeedback()}
              />
            ) : null}
            <PullRequestChecksRow
              feedback={prFeedback}
              loading={prFeedbackLoading}
              onRefresh={() => void loadPrFeedback()}
              onFix={
                !prFeedbackError && onFillInput && failedChecks.length > 0
                  ? (checks) => fillFixPrompt(buildChecksFixPrompt(currentPr, checks))
                  : undefined
              }
            />
            {mergeConflicts ? (
              <PullRequestConflictRow
                onFix={
                  !prFeedbackError && onFillInput
                    ? () => fillFixPrompt(buildMergeConflictFixPrompt(currentPr))
                    : undefined
                }
              />
            ) : null}
            <PullRequestCommentsRow
              comments={unresolvedReviewComments}
              reviews={actionableReviews}
              loading={prFeedbackLoading}
              truncated={prFeedback.commentsTruncated}
              error={prFeedback.commentsError}
              onRefresh={() => void loadPrFeedback()}
              onFix={
                !prFeedbackError && onFillInput && (unresolvedReviewComments.length > 0 || actionableReviews.length > 0)
                  ? (comments, reviews) => fillFixPrompt(
                      buildCommentsFixPrompt(currentPr, comments, reviews),
                    )
                  : undefined
              }
            />
          </>
        ) : null}
      </div>
      {state.progress ? (
        <div className="mt-1.5 flex items-center gap-2 px-2 text-[11px] text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          <span className="truncate">
            {state.progress.message || state.progress.stage}
          </span>
        </div>
      ) : null}

      <Dialog open={branchOpen} onOpenChange={setBranchOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader><DialogTitle>{t("workspace.git.selectBranch", "选择分支")}</DialogTitle><DialogDescription>{dirty ? t("workspace.git.dirtySwitchBlocked", "当前有未提交改动，只能创建新分支；切换其他分支前请先处理改动。") : t("workspace.git.branchHint", "选择本地或远端跟踪分支。")}</DialogDescription></DialogHeader>
          <div className="relative"><Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" /><Input value={branchQuery} onChange={(event) => setBranchQuery(event.target.value)} className="pl-8" placeholder={t("workspace.git.searchBranches", "搜索分支")} /></div>
          <div className="max-h-64 space-y-1 overflow-auto rounded-md border p-1">
            {branchGroups.map((group) =>
              group.branches.length > 0 ? (
                <div key={group.kind}>
                  <div className="px-2 py-1 text-[11px] font-medium text-muted-foreground">
                    {group.label}
                  </div>
                  {group.branches.map((branch) => (
                    <button key={branch.fullRef} type="button" data-ha-title-tip={branch.isCheckedOut && !branch.isCurrent ? t("workspace.git.checkedOutAt", "已在 {{path}} 检出", { path: branch.checkedOutPath || t("workspace.git.anotherWorktree", "其他工作树") }) : undefined} className={cn("flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-secondary", (branch.isCurrent || branch.isCheckedOut || dirty) && "opacity-50")} disabled={branch.isCurrent || branch.isCheckedOut || dirty || busy} onClick={() => switchBranch(branch.fullRef)}>
                      <GitBranch className="h-3.5 w-3.5" /><span className="min-w-0 flex-1 truncate">{branch.name}</span>{branch.isCurrent ? <Check className="h-4 w-4" /> : null}
                    </button>
                  ))}
                </div>
              ) : null,
            )}
          </div>
          <div className="flex gap-2 border-t pt-3">
            <Input
              value={newBranch}
              onChange={(event) => setNewBranch(event.target.value)}
              className="min-w-0 flex-1"
              placeholder="hope-agent/feature-name"
            />
            <Button
              className="shrink-0 whitespace-nowrap px-4"
              onClick={createBranch}
              disabled={!newBranch.trim() || busy}
            >
              {t("workspace.git.create", "创建")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={commitOpen} onOpenChange={setCommitOpen}>
        <DialogContent>
          <DialogHeader><DialogTitle>{t("workspace.git.commitTitle", "提交变更")}</DialogTitle><DialogDescription>{t("workspace.git.commitHint", "默认只提交已暂存内容；可选择同时暂存当前全部变更。")}</DialogDescription></DialogHeader>
          <Input value={commitSubject} onChange={(event) => setCommitSubject(event.target.value)} placeholder={t("workspace.git.commitSubject", "提交标题")} />
          <Textarea value={commitBody} onChange={(event) => setCommitBody(event.target.value)} placeholder={t("workspace.git.commitBody", "说明（可选）")} />
          <ToggleRow label={t("workspace.git.stageAll", "同时暂存全部变更")} checked={stageAll} onCheckedChange={setStageAll} />
          <ToggleRow label={t("workspace.git.pushAfter", "提交后推送")} checked={pushAfter} onCheckedChange={setPushAfter} />
          <DialogFooter><Button variant="outline" onClick={() => setCommitOpen(false)}>{t("common.cancel", "取消")}</Button><Button onClick={submitCommit} disabled={!commitSubject.trim() || busy}>{t("workspace.git.commit", "提交")}</Button></DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={prOpen} onOpenChange={setPrOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {prPushRequired
                ? t("workspace.git.pushAndCreatePr", "推送并创建拉取请求")
                : t("workspace.git.createPr", "创建拉取请求")}
            </DialogTitle>
            <DialogDescription>
              {prPushRequired
                ? t(
                    "workspace.git.pushAndCreatePrHint",
                    "将先推送当前分支并设置 upstream，再创建拉取请求；未提交的本地内容不会进入拉取请求。",
                  )
                : t(
                    "workspace.git.prHint",
                    "未提交的本地内容不会进入拉取请求；需要时可先推送当前分支。",
                  )}
            </DialogDescription>
          </DialogHeader>
          <Input value={prTitle} onChange={(event) => setPrTitle(event.target.value)} placeholder={t("workspace.git.prTitle", "标题")} />
          <Input value={prBase} onChange={(event) => setPrBase(event.target.value)} placeholder={t("workspace.git.prBase", "目标分支")} />
          <Textarea value={prBody} onChange={(event) => setPrBody(event.target.value)} placeholder={t("workspace.git.prBody", "说明（可选）")} />
          {dirty ? <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">{t("workspace.git.prDirtyWarning", "当前未提交内容不会进入拉取请求。")}</div> : null}
          {!prPushRequired && snapshot.sync.state === "ahead" ? (
            <ToggleRow label={t("workspace.git.pushBeforePr", "先推送当前分支")} checked={prPushFirst} onCheckedChange={setPrPushFirst} />
          ) : null}
          <ToggleRow label={t("workspace.git.draftPr", "创建为草稿")} checked={prDraft} onCheckedChange={setPrDraft} />
          <DialogFooter><Button variant="outline" onClick={() => setPrOpen(false)}>{t("common.cancel", "取消")}</Button><Button onClick={submitPullRequest} disabled={!prTitle.trim() || !prBase.trim() || busy}>{prPushRequired ? t("workspace.git.pushAndCreatePr", "推送并创建拉取请求") : t("workspace.git.createPr", "创建拉取请求")}</Button></DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

function GitRow({ icon: Icon, label, value, trailing, onClick, disabled, loading }: { icon: LucideIcon; label: string; value?: string; trailing?: ReactNode; onClick?: () => void; disabled?: boolean; loading?: boolean }) {
  const content = <><Icon className="h-4 w-4 shrink-0" /><span className="min-w-0 flex-1 truncate text-left text-sm font-medium">{label}</span>{value ? <span className="max-w-[45%] truncate text-xs text-muted-foreground">{value}</span> : null}{loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : trailing}</>
  if (!onClick) return <div className="flex h-9 items-center gap-2 rounded-lg px-2.5">{content}</div>
  return <button type="button" className="flex h-9 w-full items-center gap-2 rounded-lg px-2.5 transition-colors hover:bg-secondary disabled:cursor-not-allowed disabled:opacity-45" onClick={onClick} disabled={disabled}>{content}</button>
}

function PullRequestChecksRow({
  feedback,
  loading,
  onRefresh,
  onFix,
}: {
  feedback: GitPullRequestFeedback
  loading: boolean
  onRefresh: () => void
  onFix?: (checks: GitPullRequestCheck[]) => void
}) {
  const { t } = useTranslation()
  const failed = feedback.checks.filter(
    (check) => check.bucket === "fail" || check.bucket === "cancel",
  )
  const checksUnavailable = Boolean(feedback.checksError && feedback.checks.length === 0)
  const label = checksUnavailable
    ? t("workspace.git.checksUnavailable", "检查状态不可用")
    : feedback.failedChecks > 0
      ? t("workspace.git.checksFailed", "{{count}} 项检查未通过", { count: feedback.failedChecks })
      : feedback.pendingChecks > 0
        ? t("workspace.git.checksPending", "{{count}} 项检查进行中", { count: feedback.pendingChecks })
        : feedback.checks.length > 0
          ? t("workspace.git.checksPassed", "检查已通过")
          : t("workspace.git.noChecks", "暂无检查")
  const Icon = checksUnavailable
    ? Circle
    : feedback.failedChecks > 0
      ? CircleX
      : feedback.pendingChecks > 0
        ? Clock3
        : CheckCircle2

  return (
    <DropdownMenu>
      <div className="group flex h-9 w-full items-center rounded-lg transition-colors hover:bg-secondary">
        <DropdownMenuTrigger asChild>
          <button type="button" className="flex h-full min-w-0 flex-1 items-center gap-2 px-2.5 text-left">
            <Icon className={cn(
              "h-4 w-4 shrink-0",
              checksUnavailable
                ? "text-muted-foreground"
                : feedback.failedChecks > 0
                ? "text-red-500"
                : feedback.pendingChecks > 0
                  ? "text-amber-500"
                  : "text-emerald-500",
            )} />
            <span className="min-w-0 flex-1 truncate text-sm font-medium">{label}</span>
            {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
          </button>
        </DropdownMenuTrigger>
        {onFix && failed.length > 0 ? (
          <button
            type="button"
            className="mr-2 rounded px-1.5 py-0.5 text-xs font-medium text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => onFix(failed)}
          >
            {t("workspace.git.fix", "修复")}
          </button>
        ) : null}
      </div>
      <DropdownMenuContent side="left" align="start" className="w-[min(32rem,calc(100vw-2rem))] p-0">
        <FeedbackPopoverHeader
          title={t("workspace.git.checkDetails", "检查详情")}
          loading={loading}
          onRefresh={onRefresh}
        />
        {feedback.checksError ? <FeedbackError message={feedback.checksError} /> : null}
        <div className="max-h-96 overflow-y-auto p-2">
          {feedback.checks.length > 0 ? (
            <div className="space-y-1">
              {feedback.checks.map((check, index) => (
                <CheckDetailRow
                  key={`${check.name}:${check.workflow ?? ""}:${index}`}
                  check={check}
                  onFix={
                    onFix && (check.bucket === "fail" || check.bucket === "cancel")
                      ? () => onFix([check])
                      : undefined
                  }
                />
              ))}
            </div>
          ) : (
            <FeedbackEmpty label={t("workspace.git.noChecks", "暂无检查")} />
          )}
          {feedback.checksTruncated ? (
            <FeedbackTruncated label={t("workspace.git.checksTruncated", "仅显示前 100 项检查")} />
          ) : null}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function PullRequestCommentsRow({
  comments,
  reviews,
  loading,
  truncated,
  error,
  onRefresh,
  onFix,
}: {
  comments: GitPullRequestReviewComment[]
  reviews: GitPullRequestReview[]
  loading: boolean
  truncated: boolean
  error?: string | null
  onRefresh: () => void
  onFix?: (comments: GitPullRequestReviewComment[], reviews: GitPullRequestReview[]) => void
}) {
  const { t } = useTranslation()
  const feedbackCount = comments.length + reviews.length
  return (
    <DropdownMenu>
      <div className="group flex h-9 w-full items-center rounded-lg transition-colors hover:bg-secondary">
        <DropdownMenuTrigger asChild>
          <button type="button" className="flex h-full min-w-0 flex-1 items-center gap-2 px-2.5 text-left">
            <MessageCircle className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate text-sm font-medium">
              {error && feedbackCount === 0
                ? t("workspace.git.commentsUnavailable", "Review 评论不可用")
                : t("workspace.git.reviewComments", "{{count}} 条评论", { count: feedbackCount })}
            </span>
            {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
          </button>
        </DropdownMenuTrigger>
        {onFix && feedbackCount > 0 ? (
          <button
            type="button"
            className="mr-2 rounded px-1.5 py-0.5 text-xs font-medium text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => onFix(comments, reviews)}
          >
            {t("workspace.git.fix", "修复")}
          </button>
        ) : null}
      </div>
      <DropdownMenuContent side="left" align="start" className="w-[min(34rem,calc(100vw-2rem))] p-0">
        <FeedbackPopoverHeader
          title={t("workspace.git.reviewFeedback", "Review 反馈")}
          loading={loading}
          onRefresh={onRefresh}
        />
        {error ? <FeedbackError message={error} /> : null}
        <div className="max-h-[30rem] overflow-y-auto p-2">
          {reviews.length > 0 || comments.length > 0 ? (
            <div className="space-y-2">
              {reviews.map((review) => (
                <ReviewSummaryCard
                  key={review.id || `${review.author}:${review.submittedAt ?? ""}`}
                  review={review}
                  onFix={onFix ? () => onFix([], [review]) : undefined}
                />
              ))}
              {comments.map((comment) => (
                <ReviewCommentCard
                  key={comment.threadId || comment.commentId}
                  comment={comment}
                  onFix={onFix ? () => onFix([comment], []) : undefined}
                />
              ))}
            </div>
          ) : (
            <FeedbackEmpty label={t("workspace.git.noReviewComments", "没有未解决的 Review 评论")} />
          )}
          {truncated ? (
            <FeedbackTruncated label={t("workspace.git.commentsTruncated", "评论过多，仅显示首批结果")} />
          ) : null}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function PullRequestConflictRow({ onFix }: { onFix?: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex h-9 w-full items-center gap-2 rounded-lg px-2.5 text-sm font-medium">
      <CircleX className="h-4 w-4 shrink-0 text-red-500" />
      <span className="min-w-0 flex-1 truncate">
        {t("workspace.git.mergeConflict", "合并冲突")}
      </span>
      {onFix ? (
        <button
          type="button"
          className="rounded px-1.5 py-0.5 text-xs font-medium text-muted-foreground hover:bg-background/70 hover:text-foreground"
          onClick={onFix}
        >
          {t("workspace.git.fix", "修复")}
        </button>
      ) : null}
    </div>
  )
}

function FeedbackPopoverHeader({
  title,
  loading,
  onRefresh,
}: {
  title: string
  loading: boolean
  onRefresh: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2 border-b px-3 py-2">
      <span className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</span>
      <button
        type="button"
        className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground"
        onClick={onRefresh}
        disabled={loading}
        aria-label={t("common.refresh", "刷新")}
      >
        <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
      </button>
    </div>
  )
}

function CheckDetailRow({ check, onFix }: { check: GitPullRequestCheck; onFix?: () => void }) {
  const { t } = useTranslation()
  const status = check.bucket === "pass"
    ? { icon: CheckCircle2, className: "text-emerald-500", label: t("workspace.git.checkPassed", "成功") }
    : check.bucket === "pending"
      ? { icon: Clock3, className: "text-amber-500", label: t("workspace.git.checkPending", "运行中") }
      : check.bucket === "skipping"
        ? { icon: Circle, className: "text-muted-foreground", label: t("workspace.git.checkSkipped", "已跳过") }
        : { icon: CircleX, className: "text-red-500", label: t("workspace.git.checkFailed", "失败") }
  const Icon = status.icon
  return (
    <div className="flex items-start gap-2 rounded-lg px-2 py-2 hover:bg-secondary/45">
      <Icon className={cn("mt-0.5 h-4 w-4 shrink-0", status.className)} />
      <div className="min-w-0 flex-1">
        <div className="flex items-start gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-medium" data-ha-title-tip={check.name}>{check.name}</span>
          <span className="shrink-0 text-xs text-muted-foreground">{status.label}</span>
        </div>
        {check.workflow || check.description ? (
          <div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
            {[check.workflow, check.description].filter(Boolean).join(" · ")}
          </div>
        ) : null}
      </div>
      {check.link ? (
        <button
          type="button"
          className="rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground"
          onClick={() => openExternalUrl(check.link!)}
          aria-label={t("workspace.git.openCheck", "打开检查")}
        >
          <ExternalLink className="h-3.5 w-3.5" />
        </button>
      ) : null}
      {onFix ? (
        <button type="button" className="rounded px-1.5 py-1 text-xs font-medium hover:bg-background" onClick={onFix}>
          {t("workspace.git.fix", "修复")}
        </button>
      ) : null}
    </div>
  )
}

function ReviewCommentCard({
  comment,
  onFix,
}: {
  comment: GitPullRequestReviewComment
  onFix?: () => void
}) {
  const { t } = useTranslation()
  const location = `${comment.path}${comment.line ? `:${comment.line}` : ""}`
  return (
    <div className="rounded-xl border border-border/65 bg-background/70 p-3">
      <div className="whitespace-pre-wrap break-words text-sm leading-5">{comment.body}</div>
      <div className="mt-2 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <span className="truncate font-mono" data-ha-title-tip={location}>{location}</span>
        <span className="ml-auto shrink-0">{comment.author}</span>
        {comment.createdAt ? <span className="shrink-0">{formatFeedbackTime(comment.createdAt)}</span> : null}
        {comment.url ? (
          <button
            type="button"
            className="rounded p-1 hover:bg-secondary hover:text-foreground"
            onClick={() => openExternalUrl(comment.url!)}
            aria-label={t("workspace.git.openComment", "打开评论")}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </button>
        ) : null}
        {onFix ? (
          <button type="button" className="rounded px-1.5 py-1 font-medium text-foreground hover:bg-secondary" onClick={onFix}>
            {t("workspace.git.fix", "修复")}
          </button>
        ) : null}
      </div>
    </div>
  )
}

function ReviewSummaryCard({
  review,
  onFix,
}: {
  review: GitPullRequestReview
  onFix?: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-xl border border-border/65 bg-background/70 p-3">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <UserRound className="h-3.5 w-3.5" />
        <span className="font-medium text-foreground">{review.author}</span>
        <span>{reviewStateLabel(review.state, t)}</span>
        {review.submittedAt ? <span className="ml-auto">{formatFeedbackTime(review.submittedAt)}</span> : null}
      </div>
      {review.body ? (
        <div className="mt-2 whitespace-pre-wrap break-words text-sm leading-5">{review.body}</div>
      ) : null}
      <div className="mt-2 flex justify-end gap-1">
        {review.url ? (
          <button
            type="button"
            className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground"
            onClick={() => openExternalUrl(review.url!)}
            aria-label={t("workspace.git.openComment", "打开评论")}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </button>
        ) : null}
        {onFix ? (
          <button
            type="button"
            className="rounded px-1.5 py-1 text-xs font-medium hover:bg-secondary"
            onClick={onFix}
          >
            {t("workspace.git.fix", "修复")}
          </button>
        ) : null}
      </div>
    </div>
  )
}

export function PullRequestDetailsContent({
  pullRequest,
  feedback,
  loading,
  refreshError,
  onClose,
  onRefresh,
  onFixAll,
  onFixChecks,
  onFixConflict,
  onFixComments,
  onEnableAutoMerge,
}: {
  pullRequest: GitPullRequestInfo | null
  feedback: GitPullRequestFeedback | null
  loading: boolean
  refreshError?: string | null
  onClose: () => void
  onRefresh: () => void
  onFixAll?: () => void
  onFixChecks?: (checks: GitPullRequestCheck[]) => void
  onFixConflict?: () => void
  onFixComments?: (
    comments: GitPullRequestReviewComment[],
    reviews: GitPullRequestReview[],
  ) => void
  onEnableAutoMerge?: () => void
}) {
  const { t } = useTranslation()
  if (!pullRequest) return null
  const checks = feedback?.checks ?? []
  const failedChecks = checks.filter((check) => check.bucket === "fail" || check.bucket === "cancel")
  const comments = feedback?.reviewComments.filter(
    (comment) => !comment.isResolved && !comment.isOutdated,
  ) ?? []
  const reviews = (pullRequest.reviews ?? []).filter(isActionableReview)
  const mergeConflicts = hasPullRequestConflicts(pullRequest)
  const reviewers = pullRequest.reviewers ?? []

  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden">
        <div className="border-b px-4 py-3">
          <div className="flex items-start gap-3">
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-sm font-semibold">{pullRequest.title}</h2>
              <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                <span>PR #{pullRequest.number}</span>
                <span className="font-mono">{pullRequest.headBranch}</span>
                <span>→</span>
                <span className="font-mono">{pullRequest.baseBranch}</span>
                <span className="text-emerald-600">+{pullRequest.additions ?? 0}</span>
                <span className="text-red-500">-{pullRequest.deletions ?? 0}</span>
              </div>
            </div>
            <button
              type="button"
              className="rounded p-1.5 text-muted-foreground hover:bg-secondary hover:text-foreground"
              onClick={() => openExternalUrl(pullRequest.url)}
              aria-label={t("workspace.git.openPullRequest", "在 GitHub 打开")}
            >
              <ExternalLink className="h-4 w-4" />
            </button>
            <button
              type="button"
              className="rounded p-1.5 text-muted-foreground hover:bg-secondary hover:text-foreground"
              onClick={onClose}
              aria-label={t("common.close", "关闭")}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-5">
          {refreshError ? (
            <div className="rounded-xl border border-red-500/25 bg-red-500/8 px-3 py-2 text-xs text-red-600 dark:text-red-300">
              <div className="font-medium">
                {t("workspace.git.prFeedbackStale", "PR 状态刷新失败，当前数据可能已过期")}
              </div>
              <div className="mt-1 break-words text-red-500/90 dark:text-red-300/90">
                {refreshError}
              </div>
            </div>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-2">
            <PrDetailStat
              icon={GitBranch}
              label={t("workspace.git.branch", "分支")}
              value={`${pullRequest.headBranch} → ${pullRequest.baseBranch}`}
            />
            <PrDetailStat
              icon={UserRound}
              label={t("workspace.git.reviewers", "审阅者")}
              value={reviewers.length > 0 ? reviewers.map((reviewer) => reviewer.login).join(", ") : t("workspace.git.noReviewers", "暂无")}
            />
            <PrDetailStat
              icon={MessageCircle}
              label={t("workspace.git.reviewStatus", "审阅状态")}
              value={pullRequest.reviewDecision ? reviewStateLabel(pullRequest.reviewDecision, t) : t("workspace.git.reviewPending", "待处理")}
            />
            <PrDetailStat
              icon={GitMerge}
              label={t("workspace.git.mergeStatus", "合并状态")}
              value={mergeConflicts
                ? t("workspace.git.mergeConflict", "合并冲突")
                : pullRequest.autoMergeEnabled
                  ? t("workspace.git.autoMergeEnabled", "已启用自动合并")
                  : t("workspace.git.mergeReady", "可继续准备合并")}
              tone={mergeConflicts ? "danger" : "neutral"}
            />
          </div>

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">{t("workspace.git.prDescription", "描述")}</h3>
            <div className="max-h-56 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border bg-secondary/15 p-4 text-sm leading-6">
              {pullRequest.body || t("workspace.git.noDescription", "暂无描述")}
            </div>
          </section>

          <section className="space-y-2">
            <div className="flex items-center gap-2">
              <h3 className="min-w-0 flex-1 text-sm font-semibold">
                {t("workspace.git.checkDetails", "检查详情")}
              </h3>
              {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
              <Button
                variant="ghost"
                size="sm"
                onClick={onRefresh}
                disabled={loading}
                aria-label={t("common.refresh", "刷新")}
              >
                <RefreshCw className="h-3.5 w-3.5" />
              </Button>
              {onFixChecks && failedChecks.length > 0 ? (
                <Button variant="outline" size="sm" onClick={() => onFixChecks(failedChecks)}>
                  {t("workspace.git.fix", "修复")}
                </Button>
              ) : null}
            </div>
            {feedback?.checksError ? <FeedbackError message={feedback.checksError} /> : null}
            <div className="rounded-xl border p-2">
              {checks.length > 0
                ? checks.map((check, index) => (
                    <CheckDetailRow
                      key={`${check.name}:${check.workflow ?? ""}:${index}`}
                      check={check}
                      onFix={onFixChecks && (check.bucket === "fail" || check.bucket === "cancel")
                        ? () => onFixChecks([check])
                        : undefined}
                    />
                  ))
                : <FeedbackEmpty label={t("workspace.git.noChecks", "暂无检查")} />}
            </div>
          </section>

          {mergeConflicts ? (
            <section className="flex items-center gap-3 rounded-xl border border-red-500/30 bg-red-500/8 p-4">
              <CircleX className="h-5 w-5 shrink-0 text-red-500" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{t("workspace.git.mergeConflict", "合并冲突")}</div>
                <div className="text-xs text-muted-foreground">
                  {t("workspace.git.mergeConflictHint", "需要先把目标分支变更整合到当前分支并解决冲突。")}
                </div>
              </div>
              {onFixConflict ? (
                <Button size="sm" variant="outline" onClick={onFixConflict}>
                  {t("workspace.git.fix", "修复")}
                </Button>
              ) : null}
            </section>
          ) : null}

          <section className="space-y-2">
            <div className="flex items-center gap-2">
              <h3 className="min-w-0 flex-1 text-sm font-semibold">
                {t("workspace.git.reviewFeedback", "Review 反馈")} · {reviews.length + comments.length}
              </h3>
              {onFixComments && (reviews.length > 0 || comments.length > 0) ? (
                <Button variant="outline" size="sm" onClick={() => onFixComments(comments, reviews)}>
                  {t("workspace.git.fixAll", "全部修复")}
                </Button>
              ) : null}
            </div>
            {feedback?.commentsError ? <FeedbackError message={feedback.commentsError} /> : null}
            {reviews.length > 0 || comments.length > 0 ? (
              <div className="space-y-2">
                {reviews.map((review) => (
                  <ReviewSummaryCard
                    key={review.id || `${review.author}:${review.submittedAt ?? ""}`}
                    review={review}
                    onFix={onFixComments ? () => onFixComments([], [review]) : undefined}
                  />
                ))}
                {comments.map((comment) => (
                  <ReviewCommentCard
                    key={comment.threadId || comment.commentId}
                    comment={comment}
                    onFix={onFixComments ? () => onFixComments([comment], []) : undefined}
                  />
                ))}
              </div>
            ) : (
              <FeedbackEmpty label={t("workspace.git.noReviewComments", "没有未解决的 Review 评论")} />
            )}
          </section>
        </div>

        <div className="flex flex-wrap items-center justify-end gap-2 border-t px-4 py-3">
          {onFixAll ? (
            <Button variant="outline" onClick={onFixAll}>
              <WandSparkles className="mr-2 h-4 w-4" />
              {t("workspace.git.fixPullRequest", "修复 PR")}
            </Button>
          ) : null}
          {pullRequest.autoMergeEnabled ? (
            <span className="self-center text-sm text-muted-foreground">
              {t("workspace.git.autoMergeEnabled", "已启用自动合并")}
            </span>
          ) : onEnableAutoMerge ? (
            <Button onClick={onEnableAutoMerge}>
              <GitMerge className="mr-2 h-4 w-4" />
              {t("workspace.git.enableAutoMerge", "启用自动合并")}
            </Button>
          ) : null}
        </div>
    </div>
  )
}

function PrDetailStat({
  icon: Icon,
  label,
  value,
  tone = "neutral",
}: {
  icon: LucideIcon
  label: string
  value: string
  tone?: "neutral" | "danger"
}) {
  return (
    <div className="flex items-start gap-2 rounded-xl border p-3">
      <Icon className={cn("mt-0.5 h-4 w-4 shrink-0", tone === "danger" ? "text-red-500" : "text-muted-foreground")} />
      <div className="min-w-0">
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className={cn("mt-0.5 truncate text-sm font-medium", tone === "danger" && "text-red-600 dark:text-red-300")}>{value}</div>
      </div>
    </div>
  )
}

function FeedbackError({ message }: { message: string }) {
  return <div className="border-b border-red-500/20 bg-red-500/8 px-3 py-2 text-xs text-red-600 dark:text-red-300">{message}</div>
}

function FeedbackEmpty({ label }: { label: string }) {
  return <div className="px-3 py-8 text-center text-sm text-muted-foreground">{label}</div>
}

function FeedbackTruncated({ label }: { label: string }) {
  return <div className="px-2 pt-2 text-center text-[11px] text-muted-foreground">{label}</div>
}

function formatFeedbackTime(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString([], { month: "short", day: "numeric" })
}

function reviewStateLabel(
  state: string,
  t: (key: string, fallback: string) => string,
): string {
  switch (state) {
    case "APPROVED":
      return t("workspace.git.reviewApproved", "已批准")
    case "CHANGES_REQUESTED":
      return t("workspace.git.reviewChangesRequested", "要求修改")
    case "COMMENTED":
      return t("workspace.git.reviewCommented", "已评论")
    case "REVIEW_REQUIRED":
      return t("workspace.git.reviewRequired", "需要审阅")
    default:
      return state || t("workspace.git.reviewPending", "待处理")
  }
}

function ToggleRow({ label, checked, onCheckedChange }: { label: string; checked: boolean; onCheckedChange: (value: boolean) => void }) {
  return <label className="flex items-center justify-between gap-3 rounded-md border px-3 py-2 text-sm"><span>{label}</span><Switch checked={checked} onCheckedChange={onCheckedChange} /></label>
}
