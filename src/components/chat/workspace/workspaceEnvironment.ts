import type { TFunction } from "i18next"
import type {
  WorkspaceEnvironmentSnapshot,
  WorkspaceGitSnapshot,
  WorkspaceWorkingDirSource,
} from "@/lib/transport"

export type WorkspaceEnvironmentKind =
  | "noWorkingDir"
  | "missingWorkingDir"
  | "nonGit"
  | "conflicts"
  | "dirty"
  | "ahead"
  | "behind"
  | "diverged"
  | "clean"
  | "unknown"

export interface WorkspaceEnvironmentStatus {
  kind: WorkspaceEnvironmentKind
  labelKey: string
  fallback: string
  tone: "muted" | "good" | "warn" | "danger" | "info"
}

export function resolveWorkspaceEnvironmentStatus(
  snapshot: WorkspaceEnvironmentSnapshot | null,
  fallbackWorkingDir?: string | null,
  loadError?: boolean,
): WorkspaceEnvironmentStatus {
  const path = snapshot?.workingDir.path ?? fallbackWorkingDir ?? null
  if (!path) {
    return {
      kind: "noWorkingDir",
      labelKey: "workspace.environment.status.noWorkingDir",
      fallback: "无工作目录",
      tone: "muted",
    }
  }
  if (snapshot?.workingDir.exists === false) {
    return {
      kind: "missingWorkingDir",
      labelKey: "workspace.environment.status.missingWorkingDir",
      fallback: "目录不可用",
      tone: "danger",
    }
  }
  if (loadError && !snapshot) {
    return {
      kind: "unknown",
      labelKey: "workspace.environment.status.unknown",
      fallback: "状态未知",
      tone: "warn",
    }
  }
  if (!snapshot) {
    return {
      kind: "unknown",
      labelKey: "workspace.environment.status.unknown",
      fallback: "状态未知",
      tone: "muted",
    }
  }
  const git = snapshot?.git ?? null
  if (!git) {
    return {
      kind: "nonGit",
      labelKey: "workspace.environment.status.nonGit",
      fallback: "非 Git",
      tone: "muted",
    }
  }
  if (git.status.conflictedFiles > 0) {
    return {
      kind: "conflicts",
      labelKey: "workspace.environment.status.conflicts",
      fallback: "有冲突",
      tone: "danger",
    }
  }
  if (git.status.changedFiles > 0) {
    return {
      kind: "dirty",
      labelKey: "workspace.environment.status.dirty",
      fallback: "有变更",
      tone: "warn",
    }
  }
  if (git.sync.state === "diverged") {
    return {
      kind: "diverged",
      labelKey: "workspace.environment.status.diverged",
      fallback: "分叉",
      tone: "danger",
    }
  }
  if (git.sync.state === "behind") {
    return {
      kind: "behind",
      labelKey: "workspace.environment.status.behind",
      fallback: "需同步",
      tone: "warn",
    }
  }
  if (git.sync.state === "ahead") {
    return {
      kind: "ahead",
      labelKey: "workspace.environment.status.ahead",
      fallback: "未推送",
      tone: "info",
    }
  }
  return {
    kind: "clean",
    labelKey: "workspace.environment.status.clean",
    fallback: "干净",
    tone: "good",
  }
}

export function formatGitRef(t: TFunction, git: WorkspaceGitSnapshot): string {
  if (git.branch) return git.branch
  if (git.head) return t("workspace.environment.headWithHash", "HEAD {{hash}}", { hash: git.head })
  return t("workspace.environment.headOnly", "HEAD")
}

export function formatGitChanges(git: WorkspaceGitSnapshot): {
  summary: string
  delta: string | null
} {
  const count = git.status.changedFiles
  const summary = count === 1 ? "1 file" : `${count} files`
  const hasDelta = git.status.linesAdded > 0 || git.status.linesRemoved > 0
  return {
    summary,
    delta: hasDelta ? `+${git.status.linesAdded} -${git.status.linesRemoved}` : null,
  }
}

export function formatGitSync(git: WorkspaceGitSnapshot): string | null {
  const { sync } = git
  switch (sync.state) {
    case "ahead":
      return `ahead ${sync.ahead}`
    case "behind":
      return `behind ${sync.behind}`
    case "diverged":
      return `ahead ${sync.ahead} / behind ${sync.behind}`
    case "upToDate":
      return sync.upstream ? "up to date" : null
    case "noUpstream":
      return "no upstream"
    case "unknown":
      return sync.upstream ? "sync unknown" : null
  }
}

export function workingDirSourceLabelKey(source: WorkspaceWorkingDirSource): {
  key: string
  fallback: string
} {
  switch (source) {
    case "session":
      return { key: "workspace.environment.workingDirSource.session", fallback: "会话目录" }
    case "project":
      return { key: "workspace.environment.workingDirSource.project", fallback: "项目目录" }
    case "projectDefault":
      return { key: "workspace.environment.workingDirSource.projectDefault", fallback: "项目默认目录" }
    case "none":
      return { key: "workspace.environment.workingDirSource.none", fallback: "未设置" }
  }
}
