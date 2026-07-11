import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeSourceOperation =
  | "readSource"
  | "loadSourceClaims"
  | "loadSources"
  | "loadImportRuns"
  | "loadSimilarGroups"
  | "importSource"
  | "openImportHistory"
  | "retryFailedImport"
  | "dismissSimilarGroup"
  | "resolveSimilarGroup"
  | "deleteSource"
  | "reextractSource"
  | "refreshSource"
  | "loadSourceVersions"
  | "loadSourceDiff"
  | "openOriginalAsset"
  | "downloadOriginalAsset"

export type KnowledgeSourceFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeSourceErrorMessage {
  title: string
  description?: string
}

export function knowledgeSourceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeSourceErrorMessage(
  operation: KnowledgeSourceOperation,
  t: KnowledgeSourceFeedbackTranslateFn,
  error: unknown,
): KnowledgeSourceErrorMessage {
  const title = t(knowledgeSourceOperationKey(operation), {
    defaultValue: knowledgeSourceOperationFallback(operation),
  })
  const detail = knowledgeSourceErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.sources.sourceErrorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeSourceOperationKey(operation: KnowledgeSourceOperation): string {
  switch (operation) {
    case "readSource":
      return "knowledge.sources.readFailed"
    case "loadSourceClaims":
      return "knowledge.sources.sourceClaimsFailed"
    case "loadSources":
      return "knowledge.sources.sourceListFailed"
    case "loadImportRuns":
      return "knowledge.sources.importRunsListFailed"
    case "loadSimilarGroups":
      return "knowledge.sources.similarGroupsLoadFailed"
    case "importSource":
      return "knowledge.sources.importFailed"
    case "openImportHistory":
      return "knowledge.sources.importHistoryFailed"
    case "retryFailedImport":
      return "knowledge.sources.retryFailed"
    case "dismissSimilarGroup":
      return "knowledge.sources.similarDismissFailed"
    case "resolveSimilarGroup":
      return "knowledge.sources.similarResolveFailed"
    case "deleteSource":
      return "knowledge.sources.deleteFailed"
    case "reextractSource":
      return "knowledge.sources.reextractFailed"
    case "refreshSource":
      return "knowledge.sources.refreshFailed"
    case "loadSourceVersions":
      return "knowledge.sources.versionsFailed"
    case "loadSourceDiff":
      return "knowledge.sources.diffFailed"
    case "openOriginalAsset":
      return "knowledge.sources.openOriginalFailed"
    case "downloadOriginalAsset":
      return "knowledge.sources.downloadOriginalFailed"
  }
}

function knowledgeSourceOperationFallback(operation: KnowledgeSourceOperation): string {
  switch (operation) {
    case "readSource":
      return "Couldn't open source"
    case "loadSourceClaims":
      return "Couldn't load source claim references"
    case "loadSources":
      return "Couldn't load sources"
    case "loadImportRuns":
      return "Couldn't load import history"
    case "loadSimilarGroups":
      return "Couldn't load similar source groups"
    case "importSource":
      return "Couldn't import source"
    case "openImportHistory":
      return "Couldn't open import history"
    case "retryFailedImport":
      return "Couldn't retry failed imports"
    case "dismissSimilarGroup":
      return "Couldn't hide similarity suggestion"
    case "resolveSimilarGroup":
      return "Couldn't resolve duplicate sources"
    case "deleteSource":
      return "Couldn't delete source"
    case "reextractSource":
      return "Couldn't re-extract source"
    case "refreshSource":
      return "Couldn't refresh source"
    case "loadSourceVersions":
      return "Couldn't load source versions"
    case "loadSourceDiff":
      return "Couldn't load source diff"
    case "openOriginalAsset":
      return "Couldn't open original file"
    case "downloadOriginalAsset":
      return "Couldn't download original file"
  }
}
