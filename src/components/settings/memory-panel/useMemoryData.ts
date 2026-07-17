import { useState, useEffect, useCallback } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import {
  MEMORY_SOURCE_FILTER_SOURCES,
  buildMemoryBackupStructuredRestoreOptions,
  evaluateMemoryBackupPassphrase,
  type MemoryBackupBundle,
  type MemoryEncryptedBackupBundle,
  type MemoryBackupImportPreview,
  type MemoryBackupRestoreResult,
  type MemoryBackupStructuredRestoreResult,
  type MemoryEntry,
  type MemoryImportPreview,
  type MemorySearchQuery,
  type NewMemory,
  type AgentInfo,
  type MemorySourceFilter,
  type MemoryView,
} from "./types"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { useMemoryExtract } from "./useMemoryExtract"
import { useMemoryStats } from "./useMemoryStats"
import {
  isLocalModelJobTerminal,
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"
import {
  formatMemoryImportPreviewIssueMessage,
  formatMemoryImportOperationError,
  memoryImportTotal,
  showMemoryImportResultToast,
  type MemoryImportResult,
} from "./memoryImportFeedback"
import {
  memoryBackupUnlockFailureToast,
  shouldOpenMemoryBackupPreviewAfterUnlockFailure,
} from "./memoryBackupUnlockFlow"
import { memoryBackupOperationErrorToast } from "./memoryBackupOperationFeedback"
import {
  memoryCrudOperationErrorToast,
  type MemoryCrudOperationErrorToast,
  type MemoryCrudOperation,
} from "./memoryCrudOperationFeedback"
import {
  hasMemoryBackupStructuredRestorePartial,
  memoryBackupStructuredRestorePartialDescription,
  memoryBackupStructuredRestoreSummaryOptions,
} from "./memoryBackupStructuredRestoreFeedback"
import {
  hasMemoryBackupLegacyRestorePartial,
  memoryBackupLegacyRestorePartialDescription,
  memoryBackupLegacyRestoreSummaryOptions,
} from "./memoryBackupLegacyRestoreFeedback"
import { memoryEmbeddingOperationErrorToast } from "./memoryEmbeddingFeedback"

interface UseMemoryDataParams {
  agentId?: string
  isAgentMode: boolean
}

export function useMemoryData({ agentId, isAgentMode }: UseMemoryDataParams) {
  const { t } = useTranslation()
  // ── Sub-hooks ──
  const extract = useMemoryExtract({ agentId, isAgentMode })
  const statsHook = useMemoryStats()
  const reloadEmbeddingConfig = statsHook.reloadEmbeddingConfig
  const updateStats = statsHook.updateStats

  // ── Core state ──
  const [view, setView] = useState<MemoryView>("list")
  const [memories, setMemories] = useState<MemoryEntry[]>([])
  const [totalCount, setTotalCount] = useState(0)
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState("")
  const [filterType, setFilterType] = useState<string | null>(null)
  const [filterSources, setFilterSources] = useState<MemorySourceFilter[]>([])
  const [filterScope, setFilterScope] = useState<"all" | "global" | "agent">("all")
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [agentListError, setAgentListError] = useState<MemoryCrudOperationErrorToast | null>(null)
  const [statsLoadError, setStatsLoadError] = useState<MemoryCrudOperationErrorToast | null>(null)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(agentId ?? null)

  // ── Edit/Add state ──
  const [editingMemory, setEditingMemory] = useState<MemoryEntry | null>(null)
  const [formContent, setFormContent] = useState("")
  const [formType, setFormType] = useState<"user" | "feedback" | "project" | "reference">("user")
  const [formTags, setFormTags] = useState("")
  const [formScope, setFormScope] = useState<"global" | "agent">(isAgentMode ? "agent" : "global")

  // ── Multi-select state ──
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [batchLoading, setBatchLoading] = useState(false)
  const [backupLoading, setBackupLoading] = useState(false)
  const [backupPreviewLoading, setBackupPreviewLoading] = useState(false)
  const [backupRestoreLoading, setBackupRestoreLoading] = useState(false)
  const [backupPreviewOpen, setBackupPreviewOpen] = useState(false)
  const [backupPreview, setBackupPreview] = useState<MemoryBackupImportPreview | null>(null)
  const [backupPreviewContent, setBackupPreviewContent] = useState<string | null>(null)
  const [backupPreviewArchiveFile, setBackupPreviewArchiveFile] = useState<File | null>(null)
  const [backupPreviewPassphrase, setBackupPreviewPassphrase] = useState<string | null>(null)
  const [backupReviewInboxHintCount, setBackupReviewInboxHintCount] = useState(0)
  const [backupRestoreProfileConflicts, setBackupRestoreProfileConflicts] = useState(false)
  const [backupPassphraseDialogMode, setBackupPassphraseDialogMode] = useState<
    "export" | "import" | null
  >(null)
  const [backupPassphrase, setBackupPassphrase] = useState("")
  const [backupPassphraseConfirm, setBackupPassphraseConfirm] = useState("")
  const [backupPendingEncryptedContent, setBackupPendingEncryptedContent] = useState<string | null>(
    null,
  )

  // ── Reembed job tracking ──
  // TODO(dedup): this subscription + dismissReembedJob duplicate the shared
  // `useReembedJob` hook (../useReembedJob), which KnowledgePanel already uses.
  // Migrate to useReembedJob({ kind: "memory_reembed", onCompleted:
  // reloadEmbeddingConfig }) when next touching this hook — kept inline for now
  // to avoid churn on the stable memory panel.
  //
  // We watch the global LocalModelJobs stream filtered to `memory_reembed`
  // jobs so the embedding settings page can render a single status card
  // that survives navigation, refreshes, and even app restarts (interrupted
  // jobs replay through the same event channel).
  const [reembedJob, setReembedJob] = useState<LocalModelJobSnapshot | null>(null)

  // ── Import-from-AI dialog state ──
  const [importFromAIOpen, setImportFromAIOpen] = useState(false)
  const [importPreviewOpen, setImportPreviewOpen] = useState(false)
  const [importPreview, setImportPreview] = useState<MemoryImportPreview | null>(null)
  const [importPreviewContent, setImportPreviewContent] = useState<string | null>(null)
  const [importPreviewFilename, setImportPreviewFilename] = useState<string | null>(null)
  const [importPreviewLoading, setImportPreviewLoading] = useState(false)
  const [importApplyLoading, setImportApplyLoading] = useState(false)

  // ── Dedup confirmation state ──
  const [dedupSimilar, setDedupSimilar] = useState<MemoryEntry[]>([])
  const [dedupPendingEntry, setDedupPendingEntry] = useState<NewMemory | null>(null)

  const showMemoryOperationError = useCallback((
    operation: MemoryCrudOperation,
    error: unknown,
    subject?: string | null,
  ) => {
    const failureToast = memoryCrudOperationErrorToast(operation, t, error)
    const trimmedSubject = subject?.trim()
    const subjectPreview =
      trimmedSubject && trimmedSubject.length > 180
        ? `${trimmedSubject.slice(0, 177)}...`
        : trimmedSubject
    const description = [subjectPreview, failureToast.description].filter(Boolean).join("\n")
    toast.error(
      failureToast.title,
      description.length > 0 ? { description } : undefined,
    )
  }, [t])

  // ── Load agents for scope picker (standalone mode) ──
  useEffect(() => {
    if (isAgentMode) {
      setAgentListError(null)
      return
    }
    let cancelled = false
    void getTransport()
      .call<AgentInfo[]>("list_agents")
      .then((agentList) => {
        if (cancelled) return
        setAgents(agentList)
        setAgentListError(null)
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("settings", "MemoryPanel::loadAgents", "Failed to load agents", e)
        setAgentListError(memoryCrudOperationErrorToast("loadAgents", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [isAgentMode, t])

  // ── Reembed job: hydrate + subscribe ──
  useEffect(() => {
    let cancelled = false

    void getTransport()
      .call<LocalModelJobSnapshot[]>("local_model_job_list")
      .then((jobs) => {
        if (cancelled) return
        // Pick the most recent memory_reembed job. Snapshots are returned in
        // descending createdAt order so the first match is the freshest one.
        const latest = jobs.find((job) => job.kind === "memory_reembed") ?? null
        setReembedJob(latest)
      })
      .catch((e) =>
        logger.warn("settings", "MemoryPanel::loadReembedJob", "Failed to load jobs", e),
      )

    const handleSnapshot = (raw: unknown): LocalModelJobSnapshot | null => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      if (!job) return null
      if (job.kind !== "memory_reembed") return null
      setReembedJob((current) => {
        // Pick the snapshot we want to track: a new spawn replaces a terminal
        // predecessor, or a predecessor that is already cancelling; updates
        // to a job we're already tracking stay; a stale event for an unrelated
        // active job is dropped.
        const next = (() => {
          if (!current) return job
          if (current.jobId === job.jobId) return job
          if (isLocalModelJobTerminal(current)) return job
          if (current.status === "cancelling" && job.createdAt >= current.createdAt) return job
          return current
        })()
        if (next === current) return current
        // Skip the re-render when the tracked job is the same and nothing
        // observable changed — the backend emits per-batch progress and
        // status snapshots can repeat unchanged on completion.
        if (
          current &&
          next.jobId === current.jobId &&
          next.status === current.status &&
          next.phase === current.phase &&
          (next.percent ?? null) === (current.percent ?? null) &&
          (next.bytesCompleted ?? null) === (current.bytesCompleted ?? null) &&
          (next.bytesTotal ?? null) === (current.bytesTotal ?? null) &&
          (next.error ?? null) === (current.error ?? null)
        ) {
          return current
        }
        return next
      })
      return job
    }

    const handleCompletedSnapshot = (raw: unknown) => {
      const job = handleSnapshot(raw)
      if (job?.status !== "completed" || cancelled) return
      void reloadEmbeddingConfig().catch((e) =>
        logger.warn(
          "settings",
          "MemoryPanel::reloadEmbeddingAfterReembed",
          "Failed to reload embedding config",
          e,
        ),
      )
    }

    const unlistenCreated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, handleSnapshot)
    const unlistenUpdated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, handleSnapshot)
    const unlistenCompleted = getTransport().listen(
      LOCAL_MODEL_JOB_EVENTS.completed,
      handleCompletedSnapshot,
    )
    return () => {
      cancelled = true
      unlistenCreated()
      unlistenUpdated()
      unlistenCompleted()
    }
  }, [reloadEmbeddingConfig])

  const dismissReembedJob = useCallback(() => {
    const job = reembedJob
    if (!job) return
    if (!isLocalModelJobTerminal(job)) return
    void getTransport()
      .call("local_model_job_clear", { jobId: job.jobId })
      .catch((e) =>
        logger.warn("settings", "MemoryPanel::dismissReembedJob", "Failed to clear job", e),
      )
    setReembedJob(null)
  }, [reembedJob])

  // ── Build scope for queries ──
  const buildScope = useCallback((): { kind: "global" } | { kind: "agent"; id: string } | null => {
    if (isAgentMode) {
      if (filterScope === "global") return { kind: "global" }
      if (filterScope === "agent") return { kind: "agent", id: agentId! }
      return null
    }
    if (filterScope === "global") return { kind: "global" }
    if (filterScope === "agent" && selectedAgentId) return { kind: "agent", id: selectedAgentId }
    return null
  }, [isAgentMode, filterScope, agentId, selectedAgentId])

  // ── Load memories ──
  const loadMemories = useCallback(async () => {
    try {
      setLoading(true)
      const scope = buildScope()
      const sources =
        filterSources.length > 0
          ? [...new Set(filterSources.flatMap((source) => MEMORY_SOURCE_FILTER_SOURCES[source]))]
          : null

      if (searchQuery.trim()) {
        const query: MemorySearchQuery = {
          query: searchQuery,
          types: filterType ? [filterType] : null,
          sources,
          agentId: isAgentMode && filterScope === "all" ? agentId : null,
          scope: isAgentMode && filterScope === "all" ? null : scope,
          limit: 50,
        }
        const results = await getTransport().call<MemoryEntry[]>("memory_search", { query })
        setMemories(results)
      } else {
        const types = filterType ? [filterType] : null
        const results = await getTransport().call<MemoryEntry[]>("memory_list", {
          scope,
          types,
          sources,
          limit: 50,
          offset: 0,
        })
        setMemories(results)
      }
      const [count, statsResult] = await Promise.all([
        getTransport().call<number>("memory_count", { scope, sources }),
        getTransport()
          .call<import("./types").MemoryStats>("memory_stats", { scope })
          .then((data) => ({ ok: true as const, data }))
          .catch((error) => ({ ok: false as const, error })),
      ])
      setTotalCount(count)
      if (statsResult.ok) {
        updateStats(statsResult.data)
        setStatsLoadError(null)
      } else {
        logger.warn(
          "settings",
          "MemoryPanel::loadStats",
          "Failed to load memory stats",
          statsResult.error,
        )
        updateStats(null)
        setStatsLoadError(memoryCrudOperationErrorToast("loadStats", t, statsResult.error))
      }
    } catch (e) {
      logger.error("settings", "MemoryPanel::load", "Failed to load memories", e)
      showMemoryOperationError("load", e)
    } finally {
      setLoading(false)
    }
  }, [
    searchQuery,
    filterType,
    filterSources,
    buildScope,
    isAgentMode,
    filterScope,
    agentId,
    updateStats,
    t,
    showMemoryOperationError,
  ])

  useEffect(() => {
    loadMemories()
  }, [loadMemories])

  // ── CRUD handlers ──
  function buildNewMemoryEntry(): NewMemory {
    const scopeAgentId = isAgentMode ? agentId! : (selectedAgentId ?? DEFAULT_AGENT_ID)
    return {
      memoryType: formType,
      scope: formScope === "global" ? { kind: "global" } : { kind: "agent", id: scopeAgentId },
      content: formContent.trim(),
      tags: formTags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      source: "user",
    }
  }

  async function handleAdd() {
    try {
      const entry = buildNewMemoryEntry()

      const similar = await getTransport().call<MemoryEntry[]>("memory_find_similar", {
        content: entry.content,
        threshold: 0.008,
        limit: 3,
      })

      if (similar.length > 0) {
        setDedupSimilar(similar)
        setDedupPendingEntry(entry)
        return
      }

      await doAddMemory(entry)
    } catch (e) {
      logger.error("settings", "MemoryPanel::add", "Failed to add memory", e)
      showMemoryOperationError("checkDuplicate", e)
    }
  }

  async function doAddMemory(entry: NewMemory) {
    try {
      await getTransport().call("memory_add", { entry })
      setView("list")
      setFormContent("")
      setFormTags("")
      setDedupSimilar([])
      setDedupPendingEntry(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::add", "Failed to add memory", e)
      showMemoryOperationError("add", e)
    }
  }

  function handleDedupConfirm() {
    if (dedupPendingEntry) doAddMemory(dedupPendingEntry)
  }

  function handleDedupCancel() {
    setDedupSimilar([])
    setDedupPendingEntry(null)
  }

  async function handleDedupUpdate(existingId: number) {
    if (!dedupPendingEntry) return
    try {
      const existing = dedupSimilar.find((m) => m.id === existingId)
      if (!existing) return
      const mergedContent = existing.content + "\n" + dedupPendingEntry.content
      const mergedTags = [...new Set([...existing.tags, ...dedupPendingEntry.tags])]
      await getTransport().call("memory_update", {
        id: existingId,
        content: mergedContent,
        tags: mergedTags,
      })
      setView("list")
      setFormContent("")
      setFormTags("")
      setDedupSimilar([])
      setDedupPendingEntry(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::dedupUpdate", "Failed to update existing memory", e)
      showMemoryOperationError("mergeDuplicate", e)
    }
  }

  async function handleUpdate() {
    if (!editingMemory) return
    try {
      const tags = formTags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean)
      await getTransport().call("memory_update", {
        id: editingMemory.id,
        content: formContent.trim(),
        tags,
      })
      setView("list")
      setEditingMemory(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::update", "Failed to update memory", e)
      showMemoryOperationError("update", e)
    }
  }

  async function handleDelete(id: number) {
    const memoryLabel = memories.find((m) => m.id === id)?.content || t("settings.memory")
    try {
      await getTransport().call("memory_delete", { id })
      loadMemories()
      toast.success(t("common.deleted"), {
        description: memoryLabel,
      })
    } catch (e) {
      logger.error("settings", "MemoryPanel::delete", "Failed to delete memory", e)
      showMemoryOperationError("delete", e, memoryLabel)
    }
  }

  async function handleTogglePin(id: number, pinned: boolean) {
    try {
      // Optimistic update
      setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, pinned } : m)))
      await getTransport().call("memory_toggle_pin", { id, pinned })
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::togglePin", "Failed to toggle pin", e)
      showMemoryOperationError(pinned ? "pin" : "unpin", e)
      loadMemories() // Revert on error
    }
  }

  async function handleExport() {
    try {
      const md = await getTransport().call<string>("memory_export", { scope: null })
      await navigator.clipboard.writeText(md)
    } catch (e) {
      logger.error("settings", "MemoryPanel::export", "Failed to export", e)
      showMemoryOperationError("export", e)
    }
  }

  async function handleBackupExport() {
    setBackupLoading(true)
    try {
      const bundle = await getTransport().call<MemoryBackupBundle>("memory_backup_export")
      const exportedAt = bundle.exportedAt || new Date().toISOString()
      const stamp = exportedAt.replace(/[:.]/g, "-")
      const filename = `hope-agent-memory-backup-${stamp}.json`
      const blob = new Blob([JSON.stringify(bundle, null, 2)], {
        type: "application/json;charset=utf-8",
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      a.remove()
      URL.revokeObjectURL(url)

      const description = t("settings.memoryBackupExportSummary", {
        defaultValue:
          "{{memories}} memories, {{claims}} claims, {{profiles}} profile snapshots, {{episodes}} episodes, {{procedures}} procedures",
        memories: bundle.manifest.legacyMemoryCount,
        claims: bundle.manifest.claimCount,
        profiles: bundle.manifest.profileSnapshotCount,
        episodes: bundle.manifest.episodeCount ?? 0,
        procedures: bundle.manifest.procedureCount ?? 0,
      })
      if (bundle.manifest.complete) {
        toast.success(t("settings.memoryBackupExportDone", "Memory backup downloaded"), {
          description,
        })
      } else {
        toast.warning(
          t("settings.memoryBackupExportPartial", "Memory backup downloaded with warnings"),
          {
            description: bundle.manifest.warnings[0] ?? description,
          },
        )
      }
    } catch (e) {
      logger.error("settings", "MemoryPanel::backupExport", "Failed to export backup", e)
      const failureToast = memoryBackupOperationErrorToast("export", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupLoading(false)
    }
  }

  async function handleBackupExportArchive() {
    setBackupLoading(true)
    try {
      const stamp = new Date().toISOString().replace(/[:.]/g, "-")
      const result = await getTransport().exportMemoryBackupArchive(
        `hope-agent-memory-backup-${stamp}.zip`,
      )
      if (!result) return
      if (result.blob) {
        const url = URL.createObjectURL(result.blob)
        const a = document.createElement("a")
        a.href = url
        a.download = result.filename
        document.body.appendChild(a)
        a.click()
        a.remove()
        URL.revokeObjectURL(url)
      }
      toast.success(t("settings.memoryBackupArchiveExportDone", "Memory backup package exported"), {
        description: result.savedPath ?? result.filename,
      })
    } catch (e) {
      logger.error(
        "settings",
        "MemoryPanel::backupExportArchive",
        "Failed to export backup archive",
        e,
      )
      const failureToast = memoryBackupOperationErrorToast("export", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupLoading(false)
    }
  }

  async function handleBackupExportEncrypted() {
    setBackupPassphrase("")
    setBackupPassphraseConfirm("")
    setBackupPendingEncryptedContent(null)
    setBackupPassphraseDialogMode("export")
  }

  function closeBackupPassphraseDialog() {
    setBackupPassphraseDialogMode(null)
    setBackupPassphrase("")
    setBackupPassphraseConfirm("")
    setBackupPendingEncryptedContent(null)
  }

  async function submitBackupPassphraseDialog() {
    if (!backupPassphraseDialogMode) return
    const passphrase = backupPassphrase
    if (passphrase.length === 0) {
      toast.error(
        t("settings.memoryBackupEncryptedPassphraseRequired", "Enter a backup passphrase"),
      )
      return
    }
    if (backupPassphraseDialogMode === "export") {
      const policy = evaluateMemoryBackupPassphrase(passphrase)
      if (!policy.accepted) {
        toast.error(
          t(
            policy.reasonKey ?? "settings.memoryBackupEncryptedPassphraseVariety",
            policy.reasonDefault ?? "Use more variety, or a longer four-word phrase",
          ),
        )
        return
      }
      if (passphrase !== backupPassphraseConfirm) {
        toast.error(
          t("settings.memoryBackupEncryptedPassphraseMismatch", "Backup passphrases do not match"),
        )
        return
      }
      await exportEncryptedBackupWithPassphrase(passphrase)
      return
    }
    const content = backupPendingEncryptedContent
    if (!content) {
      toast.error(
        t(
          "settings.memoryBackupEncryptedUnlockContentMissing",
          "Choose the encrypted backup again before unlocking.",
        ),
      )
      closeBackupPassphraseDialog()
      return
    }
    await previewEncryptedBackupWithPassphrase(content, passphrase)
  }

  async function exportEncryptedBackupWithPassphrase(passphrase: string) {
    setBackupLoading(true)
    try {
      const bundle = await getTransport().call<MemoryEncryptedBackupBundle>(
        "memory_backup_export_encrypted",
        { passphrase },
      )
      const exportedAt = bundle.exportedAt || new Date().toISOString()
      const stamp = exportedAt.replace(/[:.]/g, "-")
      const filename = `hope-agent-memory-backup-encrypted-${stamp}.json`
      const blob = new Blob([JSON.stringify(bundle, null, 2)], {
        type: "application/json;charset=utf-8",
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = filename
      document.body.appendChild(a)
      a.click()
      a.remove()
      URL.revokeObjectURL(url)
      toast.success(
        t("settings.memoryBackupEncryptedExportDone", "Encrypted memory backup downloaded"),
      )
      closeBackupPassphraseDialog()
    } catch (e) {
      logger.error(
        "settings",
        "MemoryPanel::backupExportEncrypted",
        "Failed to export encrypted backup",
        e,
      )
      const failureToast = memoryBackupOperationErrorToast("export", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupLoading(false)
    }
  }

  async function previewEncryptedBackupWithPassphrase(content: string, passphrase: string) {
    setBackupPreviewLoading(true)
    try {
      const preview = await previewBackupContent(content, passphrase)
      if (!preview.valid) {
        const openDiagnostics = shouldOpenMemoryBackupPreviewAfterUnlockFailure(preview)
        const failureToast = memoryBackupUnlockFailureToast(preview, t)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
        setBackupPreview(preview)
        setBackupPreviewContent(content)
        setBackupPreviewArchiveFile(null)
        setBackupPreviewPassphrase(null)
        setBackupReviewInboxHintCount(0)
        if (openDiagnostics) {
          setBackupRestoreProfileConflicts(false)
          setBackupPreviewOpen(true)
          closeBackupPassphraseDialog()
        }
        return
      }
      setBackupPreview(preview)
      setBackupPreviewContent(content)
      setBackupPreviewArchiveFile(null)
      setBackupPreviewPassphrase(passphrase)
      setBackupReviewInboxHintCount(0)
      setBackupRestoreProfileConflicts(false)
      setBackupPreviewOpen(true)
      closeBackupPassphraseDialog()
      toast.success(t("settings.memoryBackupPreviewReady", "Backup preview ready"))
    } catch (e) {
      logger.error(
        "settings",
        "MemoryPanel::backupPreviewEncrypted",
        "Failed to preview encrypted backup",
        e,
      )
      const failureToast = memoryBackupOperationErrorToast("preview", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupPreviewLoading(false)
    }
  }

  async function previewBackupContent(content: string, passphrase?: string | null) {
    return getTransport().call<MemoryBackupImportPreview>("memory_backup_preview", {
      content,
      passphrase: passphrase ?? undefined,
    })
  }

  async function handleBackupPreview() {
    try {
      const input = document.createElement("input")
      input.type = "file"
      input.accept = ".json,.zip,application/json,application/zip"
      input.onchange = async () => {
        const file = input.files?.[0]
        if (!file) return
        setBackupPreviewLoading(true)
        try {
          const isArchive =
            file.name.toLowerCase().endsWith(".zip") ||
            file.type === "application/zip" ||
            file.type === "application/x-zip-compressed"
          const content = isArchive ? null : await file.text()
          const preview = isArchive
            ? ((await getTransport().previewMemoryBackupArchive(file)) as MemoryBackupImportPreview)
            : await previewBackupContent(content ?? "")
          setBackupRestoreProfileConflicts(false)
          setBackupReviewInboxHintCount(0)
          if (preview.issues.some((issue) => issue.code === "encrypted_passphrase_required")) {
            setBackupPreview(preview)
            setBackupPreviewContent(content)
            setBackupPreviewArchiveFile(null)
            setBackupPreviewPassphrase(null)
            setBackupPendingEncryptedContent(content ?? "")
            setBackupPassphrase("")
            setBackupPassphraseConfirm("")
            setBackupPassphraseDialogMode("import")
            return
          }
          setBackupPreview(preview)
          setBackupPreviewContent(content)
          setBackupPreviewArchiveFile(isArchive ? file : null)
          setBackupPreviewPassphrase(null)
          setBackupPreviewOpen(true)
          if (preview.valid) {
            toast.success(t("settings.memoryBackupPreviewReady", "Backup preview ready"))
          } else {
            toast.error(t("settings.memoryBackupPreviewInvalid", "This backup cannot be imported"))
          }
        } catch (e) {
          logger.error("settings", "MemoryPanel::backupPreview", "Failed to preview backup", e)
          const failureToast = memoryBackupOperationErrorToast("preview", t, e)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
        } finally {
          setBackupPreviewLoading(false)
        }
      }
      input.click()
    } catch (e) {
      logger.error("settings", "MemoryPanel::backupPreview", "Failed to open file picker", e)
    }
  }

  async function handleBackupRestoreLegacy() {
    if (
      (!backupPreviewContent && !backupPreviewArchiveFile) ||
      !backupPreview?.valid ||
      backupRestoreLoading
    )
      return
    setBackupRestoreLoading(true)
    try {
      const result = backupPreviewArchiveFile
        ? ((await getTransport().restoreMemoryBackupLegacyArchive(backupPreviewArchiveFile, {
            dedup: true,
          })) as MemoryBackupRestoreResult)
        : await getTransport().call<MemoryBackupRestoreResult>("memory_backup_restore_legacy", {
            content: backupPreviewContent,
            options: { dedup: true },
            passphrase: backupPreviewPassphrase ?? undefined,
          })
      setBackupPreview(result.preview)
      await loadMemories()
      toast.success(t("settings.memoryBackupRestoreDone", "Memory backup restored"), {
        description: t(
          "settings.memoryBackupRestoreSummary",
          memoryBackupLegacyRestoreSummaryOptions(result),
        ),
      })
      if (hasMemoryBackupLegacyRestorePartial(result)) {
        toast.warning(t("settings.memoryBackupRestorePartial", "Some memories were skipped"), {
          description: memoryBackupLegacyRestorePartialDescription(result, t),
        })
      }
    } catch (e) {
      logger.error("settings", "MemoryPanel::backupRestoreLegacy", "Failed to restore backup", e)
      const failureToast = memoryBackupOperationErrorToast("restoreLegacy", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupRestoreLoading(false)
    }
  }

  async function handleBackupRestoreStructured() {
    if (
      (!backupPreviewContent && !backupPreviewArchiveFile) ||
      !backupPreview?.valid ||
      backupRestoreLoading
    )
      return
    setBackupRestoreLoading(true)
    try {
      const structuredOptions = buildMemoryBackupStructuredRestoreOptions(
        backupRestoreProfileConflicts,
      )
      const result = backupPreviewArchiveFile
        ? ((await getTransport().restoreMemoryBackupStructuredArchive(
            backupPreviewArchiveFile,
            structuredOptions,
          )) as MemoryBackupStructuredRestoreResult)
        : await getTransport().call<MemoryBackupStructuredRestoreResult>(
            "memory_backup_restore_structured",
            {
              content: backupPreviewContent,
              options: structuredOptions,
              passphrase: backupPreviewPassphrase ?? undefined,
            },
          )
      setBackupPreview(result.preview)
      setBackupReviewInboxHintCount(result.restoredClaimsNeedingReview)
      await loadMemories()
      toast.success(t("settings.memoryBackupRestoreStructuredDone", "Structured memory restored"), {
        description: t(
          "settings.memoryBackupRestoreStructuredSummary",
          memoryBackupStructuredRestoreSummaryOptions(result),
        ),
      })
      if (hasMemoryBackupStructuredRestorePartial(result)) {
        toast.warning(
          t("settings.memoryBackupRestoreStructuredPartial", "Some structured items were skipped"),
          { description: memoryBackupStructuredRestorePartialDescription(result, t) },
        )
      }
    } catch (e) {
      logger.error(
        "settings",
        "MemoryPanel::backupRestoreStructured",
        "Failed to restore structured backup",
        e,
      )
      const failureToast = memoryBackupOperationErrorToast("restoreStructured", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBackupRestoreLoading(false)
    }
  }

  // ── Batch & Import handlers ──

  function toggleSelect(id: number) {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  function toggleSelectAll() {
    if (selectedIds.size === memories.length) {
      setSelectedIds(new Set())
    } else {
      setSelectedIds(new Set(memories.map((m) => m.id)))
    }
  }

  async function handleDeleteBatch() {
    if (selectedIds.size === 0) return
    setBatchLoading(true)
    const selectedCount = selectedIds.size
    try {
      await getTransport().call("memory_delete_batch", { ids: [...selectedIds] })
      setSelectedIds(new Set())
      loadMemories()
      toast.success(t("common.deleted"), {
        description: t("settings.memoryDeleteBatch", { count: selectedCount }),
      })
    } catch (e) {
      logger.error("settings", "MemoryPanel::deleteBatch", "Failed to batch delete", e)
      showMemoryOperationError(
        "deleteBatch",
        e,
        t("settings.memoryDeleteBatch", { count: selectedCount }),
      )
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleReembedBatch() {
    if (selectedIds.size === 0) return
    setBatchLoading(true)
    const selectedCount = selectedIds.size
    try {
      await getTransport().call("memory_reembed", { ids: [...selectedIds] })
      setSelectedIds(new Set())
    } catch (e) {
      logger.error("settings", "MemoryPanel::reembedBatch", "Failed to batch re-embed", e)
      showMemoryOperationError(
        "reembedSelected",
        e,
        t("settings.memoryReembed", { count: selectedCount }),
      )
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleReembedAll() {
    setBatchLoading(true)
    try {
      // Drive `reembedJob` through the `local_model_job:created` event so the
      // subscription is the single source of truth. Discarding the awaited
      // snapshot avoids a brief double-update on slow IPC.
      await getTransport().call<LocalModelJobSnapshot>("memory_reembed_start", {
        mode: "keep_existing",
      })
    } catch (e) {
      logger.error("settings", "MemoryPanel::reembedAll", "Failed to start reembed job", e)
      const failure = memoryEmbeddingOperationErrorToast("reembedStart", t, e)
      toast.error(failure.title, failure.description ? { description: failure.description } : undefined)
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleImport() {
    try {
      const input = document.createElement("input")
      input.type = "file"
      input.accept = ".json,.md,.markdown"
      input.onchange = async () => {
        const file = input.files?.[0]
        if (!file) return
        setImportPreviewLoading(true)
        const format = "auto"
        try {
          const text = await file.text()
          const preview = await getTransport().call<MemoryImportPreview>("memory_import_preview", {
            content: text,
            format,
            dedup: true,
          })
          setImportPreview(preview)
          setImportPreviewContent(text)
          setImportPreviewFilename(file.name)
          setImportPreviewOpen(true)
          if (!preview.valid) {
            toast.warning(
              (preview.issues[0]
                ? formatMemoryImportPreviewIssueMessage(t, preview.issues[0])
                : null) ||
                t("settings.memoryImportNoEntries", "No importable memories found."),
            )
            return
          }
        } catch (e) {
          logger.error("settings", "MemoryPanel::import", "Failed to import", e)
          toast.error(formatMemoryImportOperationError(t, "preview", e))
        } finally {
          setImportPreviewLoading(false)
        }
      }
      input.click()
    } catch (e) {
      logger.error("settings", "MemoryPanel::import", "Failed to open file picker", e)
    }
  }

  function closeImportPreview() {
    setImportPreviewOpen(false)
    setImportPreview(null)
    setImportPreviewContent(null)
    setImportPreviewFilename(null)
  }

  async function handleImportPreviewApply() {
    if (!importPreview?.valid || !importPreviewContent) return
    setImportApplyLoading(true)
    try {
      const result = await getTransport().call<MemoryImportResult>("memory_import", {
        content: importPreviewContent,
        format: importPreview.format || "auto",
        dedup: true,
      })
      logger.info(
        "settings",
        "MemoryPanel::import",
        `Import done: ${result.created} created, ${result.skippedDuplicate} skipped, ${result.failed} failed`,
      )
      if (memoryImportTotal(result) === 0) {
        toast.warning(t("settings.memoryImportNoEntries", "No importable memories found."))
        return
      }
      showMemoryImportResultToast(t, result, importPreview)
      closeImportPreview()
      await loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::importApply", "Failed to import", e)
      toast.error(formatMemoryImportOperationError(t, "apply", e))
    } finally {
      setImportApplyLoading(false)
    }
  }

  function startEdit(mem: MemoryEntry) {
    setEditingMemory(mem)
    setFormContent(mem.content)
    setFormType(mem.memoryType)
    setFormTags(mem.tags.join(", "))
    setView("edit")
  }

  function startAdd() {
    setFormContent("")
    setFormType("user")
    setFormTags("")
    setFormScope("global")
    setView("add")
  }

  return {
    // Core state
    view,
    setView,
    memories,
    totalCount,
    loading,
    searchQuery,
    setSearchQuery,
    filterType,
    setFilterType,
    filterSources,
    setFilterSources,
    filterScope,
    setFilterScope,
    agents,
    agentListError,
    statsLoadError,
    selectedAgentId,
    setSelectedAgentId,

    // Edit/Add state
    editingMemory,
    setEditingMemory,
    formContent,
    setFormContent,
    formType,
    setFormType,
    formTags,
    setFormTags,
    formScope,
    setFormScope,

    // Auto-extract state (from sub-hook)
    globalExtract: extract.globalExtract,
    agentExtractOverride: extract.agentExtractOverride,
    extractConfigLoaded: extract.extractConfigLoaded,
    extractConfigError: extract.extractConfigError,
    reloadExtractConfig: extract.reloadExtractConfig,
    availableProviders: extract.availableProviders,
    effectiveAutoExtract: extract.effectiveAutoExtract,
    effectiveProviderId: extract.effectiveProviderId,
    effectiveModelId: extract.effectiveModelId,
    effectiveTokenThreshold: extract.effectiveTokenThreshold,
    effectiveTimeThresholdSecs: extract.effectiveTimeThresholdSecs,
    effectiveMessageThreshold: extract.effectiveMessageThreshold,
    effectiveIdleTimeoutSecs: extract.effectiveIdleTimeoutSecs,
    effectiveMemoryEnabled: extract.effectiveMemoryEnabled,
    effectiveMemoryLearningMode: extract.effectiveMemoryLearningMode,
    agentHasOverride: extract.agentHasOverride,

    // Multi-select state
    selectedIds,
    batchLoading,
    backupLoading,
    backupPreviewLoading,
    backupRestoreLoading,
    backupPreviewOpen,
    setBackupPreviewOpen,
    backupPreview,
    backupReviewInboxHintCount,
    backupRestoreProfileConflicts,
    setBackupRestoreProfileConflicts,
    backupPassphraseDialogMode,
    backupPassphrase,
    setBackupPassphrase,
    backupPassphraseConfirm,
    setBackupPassphraseConfirm,
    closeBackupPassphraseDialog,
    submitBackupPassphraseDialog,

    // Import-from-AI dialog
    importFromAIOpen,
    setImportFromAIOpen,
    importPreviewOpen,
    setImportPreviewOpen,
    importPreview,
    importPreviewFilename,
    importPreviewLoading,
    importApplyLoading,
    closeImportPreview,
    handleImportPreviewApply,

    // Dedup state
    dedupSimilar,
    dedupPendingEntry,

    // Embedding config state (from sub-hook)
    embeddingConfig: statsHook.embeddingConfig,
    embeddingModels: statsHook.embeddingModels,
    setEmbeddingModels: statsHook.setEmbeddingModels,
    embeddingTemplates: statsHook.embeddingTemplates,
    memoryEmbeddingState: statsHook.memoryEmbeddingState,
    setMemoryEmbeddingState: statsHook.setMemoryEmbeddingState,
    embeddingConfigError: statsHook.embeddingConfigError,
    reloadEmbeddingConfig: statsHook.reloadEmbeddingConfig,

    // Dedup config state (from sub-hook)
    dedupConfig: statsHook.dedupConfig,
    setDedupConfig: statsHook.setDedupConfig,
    dedupExpanded: statsHook.dedupExpanded,
    setDedupExpanded: statsHook.setDedupExpanded,

    // Stats state (from sub-hook)
    stats: statsHook.stats,

    // Handlers
    loadMemories,
    handleAdd,
    handleDedupConfirm,
    handleDedupCancel,
    handleDedupUpdate,
    handleUpdate,
    handleDelete,
    handleTogglePin,
    handleExport,
    handleBackupExport,
    handleBackupExportArchive,
    handleBackupExportEncrypted,
    handleBackupPreview,
    handleBackupRestoreLegacy,
    handleBackupRestoreStructured,
    toggleSelect,
    toggleSelectAll,
    handleDeleteBatch,
    handleReembedBatch,
    handleReembedAll,
    handleImport,
    startEdit,
    startAdd,

    // Reembed job state
    reembedJob,
    dismissReembedJob,
    handleToggleAutoExtract: extract.handleToggleAutoExtract,
    applyMemoryLearningMode: extract.applyMemoryLearningMode,
    handleUpdateExtractModel: extract.handleUpdateExtractModel,
    handleUpdateTokenThreshold: extract.handleUpdateTokenThreshold,
    handleUpdateTimeThresholdMins: extract.handleUpdateTimeThresholdMins,
    handleUpdateMessageThreshold: extract.handleUpdateMessageThreshold,
    handleUpdateIdleTimeoutMins: extract.handleUpdateIdleTimeoutMins,
    handleToggleFlushBeforeCompact: extract.handleToggleFlushBeforeCompact,
    effectiveFlushBeforeCompact: extract.effectiveFlushBeforeCompact,
    effectiveExtractClaims: extract.effectiveExtractClaims,
    handleToggleExtractClaims: extract.handleToggleExtractClaims,
    resetAgentExtract: extract.resetAgentExtract,
  }
}
