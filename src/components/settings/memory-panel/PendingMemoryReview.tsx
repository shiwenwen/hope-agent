import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Inbox, Loader2, X } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { AgentInfo } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type {
  MemoryScope,
  PendingMemoryCandidate,
  PendingMemoryCandidatePage,
} from "./types"

function scopeKey(scope?: MemoryScope | null): string {
  if (!scope) return ""
  if (scope.kind === "global") return "global"
  return `${scope.kind}:${scope.id}`
}

function scopeFromKey(key: string): MemoryScope | null {
  if (key === "global") return { kind: "global" }
  const [kind, id] = key.split(":", 2)
  if (kind === "agent" && id) return { kind: "agent", id }
  if (kind === "project" && id) return { kind: "project", id }
  return null
}

export default function PendingMemoryReview({ agents }: { agents: AgentInfo[] }) {
  const { t } = useTranslation()
  const [items, setItems] = useState<PendingMemoryCandidate[]>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(true)
  const [workingId, setWorkingId] = useState<string | null>(null)
  const [scopes, setScopes] = useState<Record<string, string>>({})

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const [page, projectList] = await Promise.all([
        getTransport().call<PendingMemoryCandidatePage>("pending_memory_list_cmd", {
          status: "pending",
          offset: 0,
          limit: 50,
        }),
        getTransport().call<ProjectMeta[]>("list_projects_cmd"),
      ])
      setItems(page.items)
      setTotal(page.total)
      setProjects(projectList)
      setScopes((current) => {
        const next = { ...current }
        for (const item of page.items) {
          next[item.id] ??= scopeKey(item.suggestedScope)
        }
        return next
      })
    } catch (error) {
      logger.error("settings", "PendingMemoryReview::load", "Failed", error)
      toast.error(t("settings.memoryV2.review.loadFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void load()
    return getTransport().listen("memory:pending_changed", () => void load())
  }, [load])

  const availableScopes = useMemo(
    () => [
      { key: "global", label: t("settings.memoryScopeGlobal") },
      ...agents.map((agent) => ({
        key: `agent:${agent.id}`,
        label: `${t("settings.memoryScopeAgent")} · ${agent.name}`,
      })),
      ...projects.map((project) => ({
        key: `project:${project.id}`,
        label: `${t("settings.memoryScopeProject")} · ${project.name}`,
      })),
    ],
    [agents, projects, t],
  )

  const approve = async (item: PendingMemoryCandidate) => {
    const scope = scopeFromKey(scopes[item.id] ?? "")
    if (!scope) return
    setWorkingId(item.id)
    try {
      await getTransport().call("pending_memory_approve_cmd", { id: item.id, scope })
      await load()
    } catch (error) {
      logger.error("settings", "PendingMemoryReview::approve", "Failed", error)
      toast.error(t("settings.memoryV2.review.approveFailed"))
    } finally {
      setWorkingId(null)
    }
  }

  const reject = async (item: PendingMemoryCandidate) => {
    setWorkingId(item.id)
    try {
      await getTransport().call("pending_memory_reject_cmd", { id: item.id })
      await load()
    } catch (error) {
      logger.error("settings", "PendingMemoryReview::reject", "Failed", error)
      toast.error(t("settings.memoryV2.review.rejectFailed"))
    } finally {
      setWorkingId(null)
    }
  }

  return (
    <section className="mb-5 rounded-lg border border-border/60 bg-card">
      <div className="flex items-center justify-between gap-3 border-b border-border/60 px-4 py-3">
        <div className="flex items-center gap-2">
          <Inbox className="h-4 w-4 text-primary" />
          <div>
            <div className="text-sm font-medium">{t("settings.memoryV2.review.title")}</div>
            <div className="text-[11px] text-muted-foreground">
              {t("settings.memoryV2.review.desc", { count: total })}
            </div>
          </div>
        </div>
        {loading && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
      </div>
      {!loading && items.length === 0 ? (
        <div className="px-4 py-8 text-center text-xs text-muted-foreground">
          {t("settings.memoryV2.review.empty")}
        </div>
      ) : (
        <div className="divide-y divide-border/60">
          {items.map((item) => {
            const selectedScope = scopes[item.id] ?? ""
            const busy = workingId === item.id
            return (
              <div key={item.id} className="space-y-2 px-4 py-3">
                <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0">
                    <p className="text-sm leading-5">{item.content}</p>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {t(`settings.memoryV2.review.reason.${item.reason}`)} ·{" "}
                      {t(`chat.memoryTrace.kind.${item.candidateKind}`)}
                    </p>
                  </div>
                  <select
                    value={selectedScope}
                    disabled={busy}
                    onChange={(event) => setScopes((current) => ({ ...current, [item.id]: event.target.value }))}
                    className="h-8 min-w-52 rounded-md border border-input bg-background px-2 text-xs"
                    aria-label={t("settings.memoryV2.review.chooseScope")}
                  >
                    <option value="">{t("settings.memoryV2.review.chooseScope")}</option>
                    {availableScopes.map((scope) => (
                      <option key={scope.key} value={scope.key}>{scope.label}</option>
                    ))}
                  </select>
                </div>
                <div className="flex justify-end gap-2">
                  <Button type="button" variant="ghost" size="sm" className="h-7" disabled={busy} onClick={() => void reject(item)}>
                    <X className="mr-1 h-3.5 w-3.5" />{t("settings.memoryV2.review.reject")}
                  </Button>
                  <Button type="button" size="sm" className="h-7" disabled={busy || !selectedScope} onClick={() => void approve(item)}>
                    {busy ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : <Check className="mr-1 h-3.5 w-3.5" />}
                    {item.reason === "core_promotion"
                      ? t("chat.memoryTrace.promoteToCore")
                      : t("settings.memoryV2.review.approve")}
                  </Button>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}
