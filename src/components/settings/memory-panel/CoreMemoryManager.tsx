import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { FilePlus2, Loader2, RefreshCw, Save, Trash2 } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { AgentInfo } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import CoreMemoryEditor from "./CoreMemoryEditor"
import type { MemoryRuntimeConfig } from "./types"

type ScopeType = "global" | "agent" | "project"

interface TopicEntry {
  fileName: string
  relativePath: string
  name: string
  description: string
  memoryType: string
  sizeBytes: number
}

interface TopicFile extends TopicEntry {
  content: string
  fileHash: string
}

interface TopicPage {
  entries: TopicEntry[]
  total: number
  offset: number
  limit: number
}

interface TopicDraft {
  fileName?: string
  fileHash?: string
  name: string
  description: string
  memoryType: string
  content: string
}

interface CoreMemoryStats {
  indexBytes: number
  estimatedTokens: number
  indexEntryCount: number
  topicCount: number
  updatedAt?: string | null
  state: string
}

const EMPTY_DRAFT: TopicDraft = {
  name: "",
  description: "",
  memoryType: "project",
  content: "",
}

export default function CoreMemoryManager({ agents }: { agents: AgentInfo[] }) {
  const { t, i18n } = useTranslation()
  const [scopeType, setScopeType] = useState<ScopeType>("global")
  const [scopeId, setScopeId] = useState("")
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [topics, setTopics] = useState<TopicEntry[]>([])
  const [topicsLoading, setTopicsLoading] = useState(false)
  const [draft, setDraft] = useState<TopicDraft | null>(null)
  const [saving, setSaving] = useState(false)
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)
  const [stats, setStats] = useState<CoreMemoryStats | null>(null)
  const [runtime, setRuntime] = useState<MemoryRuntimeConfig | null>(null)

  useEffect(() => {
    void getTransport()
      .call<ProjectMeta[]>("list_projects_cmd")
      .then(setProjects)
      .catch((error) => logger.warn("settings", "CoreMemoryManager::projects", "Failed", error))
  }, [])

  useEffect(() => {
    if (scopeType === "agent" && !scopeId) setScopeId(agents[0]?.id ?? "")
    if (scopeType === "project" && !scopeId) setScopeId(projects[0]?.id ?? "")
    if (scopeType === "global" && scopeId) setScopeId("")
  }, [agents, projects, scopeId, scopeType])

  const effectiveScopeId = scopeType === "global" ? undefined : scopeId || undefined
  const scopeReady = scopeType === "global" || !!effectiveScopeId
  const scopeArgs = useMemo(
    () => ({ scopeType, scopeId: effectiveScopeId }),
    [effectiveScopeId, scopeType],
  )
  const scopeIdentity = `${scopeType}:${effectiveScopeId ?? ""}`
  const activeScopeIdentity = useRef(scopeIdentity)
  const loadSequence = useRef(0)
  activeScopeIdentity.current = scopeIdentity

  const loadTopics = useCallback(async () => {
    const sequence = ++loadSequence.current
    const requestedScope = scopeIdentity
    if (!scopeReady) {
      setTopics([])
      setStats(null)
      setTopicsLoading(false)
      return
    }
    setTopicsLoading(true)
    try {
      const [page, scopeStats, memoryRuntime] = await Promise.all([
        getTransport().call<TopicPage>("core_memory_topic_list_cmd", {
          ...scopeArgs,
          offset: 0,
          limit: 100,
        }),
        getTransport().call<CoreMemoryStats>("core_memory_stats_cmd", scopeArgs),
        getTransport().call<MemoryRuntimeConfig>("get_memory_runtime_config"),
      ])
      if (
        sequence !== loadSequence.current ||
        requestedScope !== activeScopeIdentity.current
      ) return
      setTopics(page.entries)
      setStats(scopeStats)
      setRuntime(memoryRuntime)
    } catch (error) {
      if (
        sequence !== loadSequence.current ||
        requestedScope !== activeScopeIdentity.current
      ) return
      logger.error("settings", "CoreMemoryManager::topics", "Failed to load topics", error)
      toast.error(t("settings.memoryV2.core.topicLoadFailed"))
    } finally {
      if (
        sequence === loadSequence.current &&
        requestedScope === activeScopeIdentity.current
      ) setTopicsLoading(false)
    }
  }, [scopeArgs, scopeIdentity, scopeReady, t])

  useEffect(() => {
    setDraft(null)
    void loadTopics()
  }, [loadTopics])

  useEffect(() => getTransport().listen("memory:core_changed", (raw) => {
    const payload = raw as { scopeType?: string; scopeId?: string | null }
    if (payload.scopeType === "all" || (payload.scopeType === scopeType && (scopeType === "global" || payload.scopeId === effectiveScopeId))) {
      void loadTopics()
    }
  }), [effectiveScopeId, loadTopics, scopeType])

  const scopeBudget = runtime
    ? scopeType === "global"
      ? runtime.core.globalTokens
      : scopeType === "agent"
        ? runtime.core.agentTokens
        : runtime.core.projectTokens
    : 0
  const budgetRatio = scopeBudget > 0 && stats
    ? Math.min(100, Math.round((stats.estimatedTokens / scopeBudget) * 100))
    : 0

  const openTopic = async (entry: TopicEntry) => {
    const requestedScope = scopeIdentity
    try {
      const file = await getTransport().call<TopicFile>("core_memory_topic_read_cmd", {
        ...scopeArgs,
        fileName: entry.fileName,
      })
      if (requestedScope !== activeScopeIdentity.current) return
      setDraft({
        fileName: file.fileName,
        fileHash: file.fileHash,
        name: file.name,
        description: file.description,
        memoryType: file.memoryType,
        content: file.content,
      })
    } catch (error) {
      logger.error("settings", "CoreMemoryManager::readTopic", "Failed", error)
      toast.error(t("settings.memoryV2.core.topicLoadFailed"))
    }
  }

  const saveTopic = async () => {
    if (!draft || !draft.name.trim() || !draft.description.trim()) return
    const requestedScope = scopeIdentity
    setSaving(true)
    try {
      const file = await getTransport().call<TopicFile>("core_memory_topic_write_cmd", {
        ...scopeArgs,
        input: {
          fileName: draft.fileName,
          expectedFileHash: draft.fileHash,
          name: draft.name,
          description: draft.description,
          memoryType: draft.memoryType,
          content: draft.content,
        },
      })
      if (requestedScope !== activeScopeIdentity.current) return
      setDraft({
        fileName: file.fileName,
        fileHash: file.fileHash,
        name: file.name,
        description: file.description,
        memoryType: file.memoryType,
        content: file.content,
      })
      await loadTopics()
      toast.success(t("common.saved"))
    } catch (error) {
      logger.error("settings", "CoreMemoryManager::saveTopic", "Failed", error)
      toast.error(t("settings.memoryV2.core.topicSaveFailed"))
    } finally {
      setSaving(false)
    }
  }

  const deleteTopic = async () => {
    if (!draft?.fileName || !draft.fileHash) return
    const requestedScope = scopeIdentity
    setSaving(true)
    try {
      await getTransport().call("core_memory_topic_delete_cmd", {
        ...scopeArgs,
        fileName: draft.fileName,
        expectedFileHash: draft.fileHash,
      })
      if (requestedScope !== activeScopeIdentity.current) return
      setDraft(null)
      await loadTopics()
    } catch (error) {
      logger.error("settings", "CoreMemoryManager::deleteTopic", "Failed", error)
      toast.error(t("settings.memoryV2.core.topicDeleteFailed"))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="mb-5 space-y-4 rounded-lg border border-border/60 bg-card p-4">
      <div className="flex flex-wrap items-end gap-3">
        <div className="text-xs text-muted-foreground">
          <label htmlFor="core-memory-scope-type">
          {t("settings.memoryV2.core.scope")}
          </label>
          <Select
            value={scopeType}
            disabled={saving}
            onValueChange={(value) => {
              setScopeType(value as ScopeType)
              setScopeId("")
            }}
          >
            <SelectTrigger id="core-memory-scope-type" className="mt-1 h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="global">{t("settings.memoryScopeGlobal")}</SelectItem>
              <SelectItem value="agent">{t("settings.memoryScopeAgent")}</SelectItem>
              <SelectItem value="project">{t("settings.memoryScopeProject")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {scopeType !== "global" && (
          <div className="min-w-56 flex-1 text-xs text-muted-foreground">
            <label htmlFor="core-memory-scope-id">
              {scopeType === "agent"
                ? t("settings.memoryScopeAgent")
                : t("settings.memoryV2.core.projectLabel")}
            </label>
            <Select
              value={scopeId}
              disabled={saving || (scopeType === "agent" ? agents : projects).length === 0}
              onValueChange={setScopeId}
            >
              <SelectTrigger id="core-memory-scope-id" className="mt-1 h-8 w-full text-xs">
                <SelectValue
                  placeholder={scopeType === "agent"
                    ? t("settings.memoryExperienceNoAgents", "No agents available")
                    : t("settings.memoryExperienceNoProjects", "No projects available")}
                />
              </SelectTrigger>
              <SelectContent>
                {(scopeType === "agent" ? agents : projects).map((item) => (
                  <SelectItem key={item.id} value={item.id}>{item.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {scopeReady && stats && (
        <div className="rounded-md border border-border/60 bg-background/50 px-3 py-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span>{t("settings.memoryV2.core.actualTokens", { used: stats.estimatedTokens, total: scopeBudget })}</span>
            <span>{t("settings.memoryV2.core.stats", { entries: stats.indexEntryCount, topics: stats.topicCount })}</span>
            <span>
              {stats.updatedAt
                ? t("settings.memoryV2.core.updatedAt", {
                    value: new Date(stats.updatedAt).toLocaleString(i18n.resolvedLanguage),
                  })
                : t("settings.memoryV2.core.neverUpdated")}
            </span>
          </div>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-secondary">
            <div
              className={budgetRatio >= 100 ? "h-full bg-destructive" : budgetRatio >= 80 ? "h-full bg-amber-500" : "h-full bg-primary"}
              style={{ width: `${budgetRatio}%` }}
            />
          </div>
        </div>
      )}

      {scopeReady && (
        <CoreMemoryEditor
          key={`${scopeType}:${effectiveScopeId ?? ""}`}
          scope={scopeType}
          agentId={scopeType === "agent" ? effectiveScopeId : undefined}
          projectId={scopeType === "project" ? effectiveScopeId : undefined}
        />
      )}

      <div className="grid gap-4 lg:grid-cols-[280px_minmax(0,1fr)]">
        <div className="rounded-md border border-border/60">
          <div className="flex items-center justify-between border-b border-border/60 px-3 py-2">
            <span className="text-xs font-medium">{t("settings.memoryV2.core.topics")}</span>
            <div className="flex items-center gap-1">
              <IconTip label={t("common.refresh")}>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-7 w-7"
                  onClick={() => void loadTopics()}
                  disabled={topicsLoading || !scopeReady}
                  aria-label={t("common.refresh")}
                >
                  {topicsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                </Button>
              </IconTip>
              <IconTip label={t("project.autoMemory.newTopic")}>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-7 w-7"
                  onClick={() => setDraft({ ...EMPTY_DRAFT })}
                  disabled={!scopeReady}
                  aria-label={t("project.autoMemory.newTopic")}
                >
                  <FilePlus2 className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            </div>
          </div>
          <div className="max-h-72 overflow-y-auto p-1.5">
            {topics.length === 0 && !topicsLoading && (
              <div className="px-2 py-6 text-center text-xs text-muted-foreground">
                {t("settings.memoryV2.core.noTopics")}
              </div>
            )}
            {topics.map((entry) => (
              <button
                key={entry.fileName}
                type="button"
                onClick={() => void openTopic(entry)}
                className="block w-full rounded px-2 py-2 text-left hover:bg-secondary/60"
              >
                <div className="truncate text-xs font-medium">{entry.name}</div>
                <div className="mt-0.5 line-clamp-2 text-[11px] text-muted-foreground">{entry.description}</div>
              </button>
            ))}
          </div>
        </div>

        <div className="min-h-48 rounded-md border border-border/60 p-3">
          {!draft ? (
            <div className="flex h-full min-h-44 items-center justify-center text-xs text-muted-foreground">
              {t("settings.memoryV2.core.selectTopic")}
            </div>
          ) : (
            <div className="space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <Input
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                  placeholder={t("settings.memoryV2.core.topicName")}
                  aria-label={t("settings.memoryV2.core.topicName")}
                />
                <Select
                  value={draft.memoryType}
                  onValueChange={(memoryType) => setDraft({ ...draft, memoryType })}
                >
                  <SelectTrigger aria-label={t("settings.memoryType")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="user">{t("settings.memoryType_user")}</SelectItem>
                    <SelectItem value="feedback">{t("settings.memoryType_feedback")}</SelectItem>
                    <SelectItem value="project">{t("settings.memoryType_project")}</SelectItem>
                    <SelectItem value="reference">{t("settings.memoryType_reference")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <Input
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
                placeholder={t("settings.memoryV2.core.topicDesc")}
                aria-label={t("settings.memoryV2.core.topicDesc")}
              />
              <Textarea
                value={draft.content}
                onChange={(event) => setDraft({ ...draft, content: event.target.value })}
                placeholder={t("settings.memoryV2.core.topicContent")}
                aria-label={t("settings.memoryV2.core.topicContent")}
                className="min-h-40 font-mono text-xs"
              />
              <div className="flex justify-end gap-2">
                {draft.fileName && (
                  <Button type="button" variant="destructive" size="sm" onClick={() => setDeleteConfirmOpen(true)} disabled={saving}>
                    <Trash2 className="mr-1.5 h-3.5 w-3.5" />{t("common.delete")}
                  </Button>
                )}
                <Button type="button" size="sm" onClick={() => void saveTopic()} disabled={saving || !draft.name.trim() || !draft.description.trim()}>
                  {saving ? <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" /> : <Save className="mr-1.5 h-3.5 w-3.5" />}
                  {t("common.save")}
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>

      <AlertDialog open={deleteConfirmOpen} onOpenChange={setDeleteConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("common.delete")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.memoryV2.core.topicDeleteConfirm")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={saving}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                type="button"
                variant="destructive"
                disabled={saving}
                onClick={() => void deleteTopic()}
              >
                {saving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                {t("common.delete")}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
