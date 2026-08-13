import type { PendingFileQuote } from "@/types/chat"

export type ProjectFolderIdentity = NonNullable<PendingFileQuote["projectRoot"]>

export type ProjectFileQuoteReveal = Pick<
  PendingFileQuote,
  "path" | "name" | "startLine" | "endLine" | "projectRoot" | "worktreeRoot"
> & { nonce: number }

export interface ResolvedProjectFileQuoteTarget {
  path: string
  projectRoot: ProjectFolderIdentity | null
  worktreeRoot: string | null
  valid: boolean
}

/** Build an unambiguous model/history path for a quote captured outside the primary root. */
export function quoteReferencePath(quote: PendingFileQuote): string {
  const root = quote.worktreeRoot ?? quote.projectRoot?.path
  if (!root) return quote.path
  const separator = root.includes("\\") && !root.includes("/") ? "\\" : "/"
  const base = root.replace(/[\\/]+$/, "")
  const relative = quote.path.replace(/^[\\/]+/, "").replace(/[\\/]/g, separator)
  return `${base}${separator}${relative}`
}

/**
 * Restore the exact linked root and optional Git worktree for a quote-chip
 * jump. A carried project identity must still match the current Project row;
 * stale identities fail closed instead of redirecting the relative path into
 * the primary root. Worktrees are reopened through the backend-validated path
 * scope. Older persisted quotes with an absolute path are recovered by a
 * boundary-safe root match.
 */
export function resolveProjectFileQuoteTarget(
  quote: Pick<PendingFileQuote, "path" | "projectRoot" | "worktreeRoot">,
  linkedRootPaths: string[],
): ResolvedProjectFileQuoteTarget {
  if (quote.projectRoot) {
    const currentPath = linkedRootPaths[quote.projectRoot.index]
    if (currentPath !== quote.projectRoot.path) {
      return { path: quote.path, projectRoot: null, worktreeRoot: null, valid: false }
    }
    return {
      path: quote.path,
      projectRoot: quote.projectRoot,
      worktreeRoot: quote.worktreeRoot ?? null,
      valid: true,
    }
  }

  if (quote.worktreeRoot) {
    return { path: quote.path, projectRoot: null, worktreeRoot: quote.worktreeRoot, valid: true }
  }

  const normalizedQuotePath = normalizePath(quote.path)
  for (let index = 0; index < linkedRootPaths.length; index += 1) {
    const linkedPath = linkedRootPaths[index]
    const normalizedRoot = normalizePath(linkedPath).replace(/\/+$/, "")
    const prefix = `${normalizedRoot}/`
    if (!normalizedQuotePath.startsWith(prefix)) continue
    return {
      path: normalizedQuotePath.slice(prefix.length),
      projectRoot: { index, path: linkedPath },
      worktreeRoot: null,
      valid: true,
    }
  }

  return { path: quote.path, projectRoot: null, worktreeRoot: null, valid: true }
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/")
}
