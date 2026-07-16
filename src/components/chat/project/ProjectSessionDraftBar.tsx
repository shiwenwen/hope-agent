import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Check,
  ChevronDown,
  Folder,
  GitBranch,
  GitFork,
  Laptop,
  Loader2,
  Search,
  X,
} from "lucide-react"

import { FloatingMenu, FLOATING_MENU_ITEM_CLASS } from "@/components/ui/floating-menu"
import { SearchInput } from "@/components/ui/search-input"
import { useClickOutside } from "@/hooks/useClickOutside"
import { getTransport } from "@/lib/transport-provider"
import type { GitBranchInfo, GitInfo } from "@/lib/transport"
import { cn } from "@/lib/utils"
import type { ProjectMeta } from "@/types/project"
import {
  defaultProjectBranch,
  projectBranchDisabledForLaunch,
  projectRuntimeDraftForBranch,
  type ProjectRuntimeDraft,
} from "./projectRuntimeDraft"

export function ProjectSessionDraftBar({
  project,
  projects,
  draft,
  disabled = false,
  progressStage,
  progressError,
  onDraftChange,
  onSelectProject,
  onRemoveProject,
  onRetry,
  onUseLocal,
}: {
  project: ProjectMeta | null
  projects: ProjectMeta[]
  draft: ProjectRuntimeDraft
  disabled?: boolean
  progressStage?: string | null
  progressError?: string | null
  onDraftChange: (draft: ProjectRuntimeDraft) => void
  onSelectProject: (projectId: string, defaultAgentId?: string | null) => void
  onRemoveProject: () => void
  onRetry: () => void
  onUseLocal: () => void
}) {
  const { t } = useTranslation()
  const [gitInfo, setGitInfo] = useState<GitInfo | null>(null)
  const [gitLoading, setGitLoading] = useState(true)
  const [gitError, setGitError] = useState<string | null>(null)
  const [gitNotice, setGitNotice] = useState<string | null>(null)
  const [projectOpen, setProjectOpen] = useState(false)
  const [launchOpen, setLaunchOpen] = useState(false)
  const [branchOpen, setBranchOpen] = useState(false)
  const [projectQuery, setProjectQuery] = useState("")
  const [branchQuery, setBranchQuery] = useState("")
  const projectMenuRef = useRef<HTMLDivElement>(null)
  const launchMenuRef = useRef<HTMLDivElement>(null)
  const branchMenuRef = useRef<HTMLDivElement>(null)
  const projectId = project?.id ?? null

  useClickOutside(projectMenuRef, useCallback(() => setProjectOpen(false), []))
  useClickOutside(launchMenuRef, useCallback(() => setLaunchOpen(false), []))
  useClickOutside(branchMenuRef, useCallback(() => setBranchOpen(false), []))

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (cancelled) return
      setGitLoading(!!projectId)
      setGitError(null)
      setGitNotice(null)
      setGitInfo(null)
    })
    if (!projectId) {
      return () => {
        cancelled = true
      }
    }
    getTransport()
      .call<GitInfo | null>("project_git_info", { scope: "project", scopeId: projectId })
      .then((info) => {
        if (cancelled) return
        setGitInfo(info)
        setGitLoading(false)
      })
      .catch((error) => {
        if (cancelled) return
        setGitError(error instanceof Error ? error.message : String(error))
        setGitLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [projectId])

  useEffect(() => {
    if (!gitInfo) return
    const selected = gitInfo.branches.find((branch) => branch.fullRef === draft.baseRef)
    if (selected) {
      const expectedInclude = selected.kind === "local" && selected.isCurrent
      if (
        draft.baseRefKind !== selected.kind ||
        draft.includeLocalChanges !== expectedInclude
      ) {
        onDraftChange(projectRuntimeDraftForBranch(draft, selected))
      }
      return
    }
    const fallback = defaultProjectBranch(gitInfo)
    if (fallback) {
      if (draft.baseRef) {
        queueMicrotask(() =>
          setGitNotice(
            t("chat.projectRuntime.branchChanged", "原分支已失效，已回退到 {{branch}}", {
              branch: fallback.name,
            }),
          ),
        )
      }
      onDraftChange(projectRuntimeDraftForBranch(draft, fallback))
    }
  }, [draft, gitInfo, onDraftChange, t])

  const selectedBranch = gitInfo?.branches.find((branch) => branch.fullRef === draft.baseRef) ?? null
  const activeProjects = useMemo(
    () =>
      projects.filter(
        (candidate) =>
          !candidate.archived &&
          candidate.name.toLowerCase().includes(projectQuery.trim().toLowerCase()),
      ),
    [projectQuery, projects],
  )
  const filteredBranches = useMemo(() => {
    const query = branchQuery.trim().toLowerCase()
    return (gitInfo?.branches ?? []).filter((branch) =>
      branch.name.toLowerCase().includes(query),
    )
  }, [branchQuery, gitInfo?.branches])
  const localBranches = filteredBranches.filter((branch) => branch.kind === "local")
  const remoteBranches = filteredBranches.filter((branch) => branch.kind === "remote")
  const canUseWorktree = !!gitInfo && gitInfo.branches.length > 0
  const worktreeUnavailableReason = gitInfo
    ? t("chat.projectRuntime.noBranches", "Git 仓库中没有可用分支")
    : gitError || t("chat.projectRuntime.notGit", "项目工作目录不在 Git 仓库中")
  const dirtyCount = gitInfo?.dirty.changedFiles ?? 0
  const selectedBranchSummary = selectedBranch
    ? draft.launchMode === "worktree"
      ? draft.includeLocalChanges && dirtyCount > 0
        ? t("chat.projectRuntime.includeChanges", "将包含 {{count}} 个本地改动", {
            count: dirtyCount,
          })
        : selectedBranch.isCurrent
          ? t("chat.projectRuntime.cleanBranch", "当前分支没有未提交改动")
          : t("chat.projectRuntime.excludeChanges", "不会包含当前工作区的未提交改动")
      : selectedBranch.isCurrent
        ? dirtyCount > 0
          ? t("chat.projectRuntime.localKeepsChanges", "将在当前分支工作，保留 {{count}} 个本地改动", {
              count: dirtyCount,
            })
          : t("chat.projectRuntime.localCurrentBranch", "将在当前分支工作")
        : selectedBranch.kind === "remote"
          ? t(
              "chat.projectRuntime.localSwitchRemote",
              "将从 {{branch}} 创建跟踪分支；当前工作区必须干净",
              { branch: selectedBranch.name },
            )
          : t(
              "chat.projectRuntime.localSwitchBranch",
              "将把本地工作区切换到 {{branch}}；当前工作区必须干净",
              { branch: selectedBranch.name },
            )
    : null

  const chooseLaunchMode = (mode: "local" | "worktree") => {
    if (mode === "worktree" && !canUseWorktree) return
    const branch = selectedBranch ?? (gitInfo ? defaultProjectBranch(gitInfo) : null)
    onDraftChange(
      branch
        ? projectRuntimeDraftForBranch({ ...draft, launchMode: mode }, branch)
        : { ...draft, launchMode: mode },
    )
    setGitNotice(null)
    setLaunchOpen(false)
  }

  const renderBranchGroup = (label: string, branches: GitBranchInfo[]) => {
    if (branches.length === 0) return null
    return (
      <div className="py-1">
        <div className="px-2.5 py-1 text-[11px] font-medium text-muted-foreground">{label}</div>
        {branches.map((branch) => (
          <button
            key={branch.fullRef}
            type="button"
            disabled={projectBranchDisabledForLaunch(branch, draft.launchMode)}
            className={cn(
              FLOATING_MENU_ITEM_CLASS,
              "disabled:cursor-not-allowed disabled:opacity-45",
            )}
            onClick={() => {
              setGitNotice(null)
              onDraftChange(projectRuntimeDraftForBranch(draft, branch))
              setBranchOpen(false)
            }}
          >
            <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate">{branch.name}</span>
            {branch.isCheckedOut && !branch.isCurrent ? (
              <span className="text-[10px] text-muted-foreground">
                {t("chat.projectRuntime.checkedOut", "已在其他工作树中使用")}
              </span>
            ) : null}
            {draft.baseRef === branch.fullRef ? <Check className="h-4 w-4" /> : null}
          </button>
        ))}
      </div>
    )
  }

  const controlClass =
    "relative inline-flex min-w-0 items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-sm text-foreground transition-colors hover:bg-background/70 disabled:pointer-events-none disabled:opacity-50"

  return (
    <div className="rounded-t-input-dock border-b border-border-soft bg-surface-subtle/60 px-2 py-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-1">
        <div ref={projectMenuRef} className="relative min-w-0">
          <div className="flex min-w-0 items-center rounded-lg hover:bg-background/70">
            <button
              type="button"
              disabled={disabled}
              className={cn(controlClass, "max-w-[240px] hover:bg-transparent")}
              onClick={() => setProjectOpen((open) => !open)}
            >
              <Folder className="h-4 w-4 shrink-0" />
              <span className="truncate">
                {project?.name ?? t("chat.projectRuntime.selectProject", "选择项目")}
              </span>
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            </button>
            {project ? (
              <button
                type="button"
                disabled={disabled}
                aria-label={t("chat.projectRuntime.removeProject", "不在项目中工作")}
                data-ha-title-tip={t("chat.projectRuntime.removeProject", "不在项目中工作")}
                className="mr-1 rounded-full p-0.5 text-muted-foreground hover:bg-foreground hover:text-background disabled:pointer-events-none"
                onClick={onRemoveProject}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
          <FloatingMenu
            open={projectOpen}
            positionClassName="top-full left-0 mt-2"
            originClassName="origin-top-left"
            className="w-[300px] p-1.5"
            onEscapeKeyDown={() => setProjectOpen(false)}
          >
            <div className="relative mb-1">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 z-10 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <SearchInput
                value={projectQuery}
                onChange={(event) => setProjectQuery(event.target.value)}
                placeholder={t("chat.projectRuntime.searchProjects", "搜索项目")}
                className="h-8 pl-8 text-sm"
              />
            </div>
            <button
              type="button"
              className={FLOATING_MENU_ITEM_CLASS}
              onClick={() => {
                if (project) onRemoveProject()
                setProjectOpen(false)
              }}
            >
              {project ? <X className="h-4 w-4" /> : <Check className="h-4 w-4" />}
              {t("chat.projectRuntime.noProject", "不在项目中")}
            </button>
            <div className="max-h-64 overflow-y-auto">
              {activeProjects.map((candidate) => (
                <button
                  key={candidate.id}
                  type="button"
                  className={FLOATING_MENU_ITEM_CLASS}
                  onClick={() => {
                    onSelectProject(candidate.id, candidate.defaultAgentId)
                    setProjectOpen(false)
                  }}
                >
                  <Folder className="h-4 w-4 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate">{candidate.name}</span>
                  {candidate.id === project?.id ? <Check className="h-4 w-4" /> : null}
                </button>
              ))}
            </div>
          </FloatingMenu>
        </div>

        <div ref={launchMenuRef} className={cn("relative", !project && "hidden")}>
          <button
            type="button"
            disabled={disabled}
            className={controlClass}
            onClick={() => setLaunchOpen((open) => !open)}
          >
            {draft.launchMode === "worktree" ? (
              <GitFork className="h-4 w-4" />
            ) : (
              <Laptop className="h-4 w-4" />
            )}
            <span>
              {draft.launchMode === "worktree"
                ? t("chat.projectRuntime.worktree", "新工作树")
                : t("chat.projectRuntime.local", "本地处理")}
            </span>
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          </button>
          <FloatingMenu
            open={launchOpen}
            positionClassName="top-full left-0 mt-2"
            originClassName="origin-top-left"
            className="w-[260px] p-1.5"
            onEscapeKeyDown={() => setLaunchOpen(false)}
          >
            <button type="button" className={FLOATING_MENU_ITEM_CLASS} onClick={() => chooseLaunchMode("local")}>
              <Laptop className="h-4 w-4" />
              <span className="flex-1">{t("chat.projectRuntime.local", "本地处理")}</span>
              {draft.launchMode === "local" ? <Check className="h-4 w-4" /> : null}
            </button>
            <button
              type="button"
              disabled={!canUseWorktree || gitLoading}
              className={cn(FLOATING_MENU_ITEM_CLASS, "disabled:cursor-not-allowed disabled:opacity-45")}
              onClick={() => chooseLaunchMode("worktree")}
            >
              {gitLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <GitFork className="h-4 w-4" />}
              <span className="flex-1">{t("chat.projectRuntime.worktree", "新工作树")}</span>
              {draft.launchMode === "worktree" ? <Check className="h-4 w-4" /> : null}
            </button>
            {!gitLoading && !canUseWorktree ? (
              <p className="px-2.5 py-1.5 text-[11px] text-muted-foreground">
                {worktreeUnavailableReason}
              </p>
            ) : null}
          </FloatingMenu>
        </div>

        {project && (gitLoading || gitInfo) ? (
          <div ref={branchMenuRef} className="relative min-w-0">
            <button
              type="button"
              disabled={disabled || gitLoading}
              data-ha-title-tip={selectedBranchSummary ?? undefined}
              className={cn(controlClass, "max-w-[300px]")}
              onClick={() => setBranchOpen((open) => !open)}
            >
              {gitLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <GitBranch className="h-4 w-4" />}
              <span className="truncate">
                {selectedBranch?.name ?? t("chat.projectRuntime.selectBranch", "选择分支")}
              </span>
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            </button>
            <FloatingMenu
              open={branchOpen}
              positionClassName="top-full left-0 mt-2"
              originClassName="origin-top-left"
              className="w-[360px] p-1.5"
              onEscapeKeyDown={() => setBranchOpen(false)}
            >
              <div className="relative mb-1">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 z-10 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <SearchInput
                  value={branchQuery}
                  onChange={(event) => setBranchQuery(event.target.value)}
                  placeholder={t("chat.projectRuntime.searchBranches", "搜索分支")}
                  className="h-8 pl-8 text-sm"
                />
              </div>
              <div className="max-h-72 overflow-y-auto">
                {renderBranchGroup(t("chat.projectRuntime.localBranches", "本地分支"), localBranches)}
                {renderBranchGroup(t("chat.projectRuntime.remoteBranches", "远端分支"), remoteBranches)}
              </div>
            </FloatingMenu>
          </div>
        ) : null}
      </div>

      {project && gitNotice ? (
        <div className="px-2.5 pb-0.5 pt-1 text-[11px] text-amber-600 dark:text-amber-400">
          {gitNotice}
        </div>
      ) : null}

      {project && progressStage ? (
        <div
          className={cn(
            "flex items-center gap-2 px-2.5 pb-0.5 pt-1 text-xs",
            progressError ? "text-destructive" : "text-primary",
          )}
        >
          <span className="min-w-0 flex-1">
            {progressError || t(`chat.projectRuntime.stages.${progressStage}`, progressStage)}
          </span>
          {progressError ? (
            <>
              <button
                type="button"
                disabled={disabled}
                className="font-medium hover:underline disabled:opacity-50"
                onClick={onRetry}
              >
                {t("common.retry", "重试")}
              </button>
              {draft.launchMode === "worktree" ? (
                <button
                  type="button"
                  disabled={disabled}
                  className="font-medium hover:underline disabled:opacity-50"
                  onClick={onUseLocal}
                >
                  {t("chat.projectRuntime.useLocal", "改为本地处理")}
                </button>
              ) : null}
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
