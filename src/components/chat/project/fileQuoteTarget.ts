import type { PendingFileQuote } from "@/types/chat"

export type ProjectFolderIdentity = NonNullable<PendingFileQuote["projectRoot"]>

export type ProjectFileQuoteReveal = Pick<
  PendingFileQuote,
  "path" | "name" | "startLine" | "endLine" | "projectRoot"
> & { nonce: number }

export interface ResolvedProjectFileQuoteTarget {
  path: string
  projectRoot: ProjectFolderIdentity | null
  valid: boolean
}

/** Build an unambiguous model/history path for a quote captured in a linked root. */
export function quoteReferencePath(quote: PendingFileQuote): string {
  const root = quote.projectRoot?.path
  if (!root) return quote.path
  const separator = root.includes("\\") && !root.includes("/") ? "\\" : "/"
  const base = root.replace(/[\\/]+$/, "")
  const relative = quote.path.replace(/^[\\/]+/, "").replace(/[\\/]/g, separator)
  return `${base}${separator}${relative}`
}

/**
 * Restore the exact linked root for a quote-chip jump. A carried identity must
 * still match the current Project row; stale identities fail closed instead
 * of redirecting the relative path into the primary root. Older persisted
 * quotes with an absolute path are recovered by a boundary-safe root match.
 */
export function resolveProjectFileQuoteTarget(
  quote: Pick<PendingFileQuote, "path" | "projectRoot">,
  linkedRootPaths: string[],
): ResolvedProjectFileQuoteTarget {
  if (quote.projectRoot) {
    const currentPath = linkedRootPaths[quote.projectRoot.index]
    if (currentPath !== quote.projectRoot.path) {
      return { path: quote.path, projectRoot: null, valid: false }
    }
    return { path: quote.path, projectRoot: quote.projectRoot, valid: true }
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
      valid: true,
    }
  }

  return { path: quote.path, projectRoot: null, valid: true }
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/")
}
