import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Library, Loader2, Lock } from "lucide-react"

import { Label } from "@/components/ui/label"
import { KbAccessControl } from "@/components/knowledge/KbAccessControl"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { KbAccess, KbAttachment, KnowledgeBaseMeta } from "@/types/knowledge"

/**
 * Project-level knowledge-space attach control (design D10). Mirrors the chat
 * input's session-scoped `KnowledgePicker` but binds to a *project* via the
 * owner-plane `attach_project_kb_cmd` / `detach_project_kb_cmd` / `list_project_kbs_cmd`.
 * Project attaches flow into `effective_kb_access` as the project leg of
 * `max(session, project)`, so any session in this project can cite these notes.
 * Edit-mode only — a project id must already exist to bind to.
 */
export default function ProjectKnowledgeSection({ projectId }: { projectId: string }) {
  const { t } = useTranslation()
  const [kbs, setKbs] = useState<KnowledgeBaseMeta[]>([])
  const [attachments, setAttachments] = useState<KbAttachment[]>([])
  const [loading, setLoading] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)

  const load = useCallback(() => {
    setLoading(true)
    Promise.all([
      getTransport().call<KnowledgeBaseMeta[]>("list_kbs_cmd", { includeArchived: false }),
      getTransport().call<KbAttachment[]>("list_project_kbs_cmd", { projectId }),
    ])
      .then(([all, att]) => {
        setKbs(all)
        setAttachments(att)
      })
      .catch((e) => logger.error("project", "ProjectKnowledgeSection::load", "load failed", e))
      .finally(() => setLoading(false))
  }, [projectId])

  useEffect(() => {
    load()
  }, [load])

  // React to KB mutations made elsewhere (knowledge view, agent tools, watcher).
  useEffect(() => getTransport().listen("knowledge:changed", load), [load])

  const attachmentFor = (id: string) => attachments.find((a) => a.id === id)

  const setAttach = useCallback(
    async (kb: KnowledgeBaseMeta, access: KbAccess | null) => {
      setBusyId(kb.id)
      try {
        if (access === null) {
          await getTransport().call("detach_project_kb_cmd", { projectId, kbId: kb.id })
        } else {
          await getTransport().call("attach_project_kb_cmd", { projectId, kbId: kb.id, access })
        }
        load()
      } catch (e) {
        logger.error("project", "ProjectKnowledgeSection::setAttach", "attach/detach failed", e)
      } finally {
        setBusyId(null)
      }
    },
    [projectId, load],
  )

  return (
    <div className="space-y-1.5">
      <Label className="flex items-center gap-2">
        <Library className="h-4 w-4 text-muted-foreground" />
        {t("project.knowledge.label", "Knowledge spaces")}
        {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
      </Label>
      <p className="text-xs text-muted-foreground">
        {t(
          "project.knowledge.hint",
          "Attached spaces let this project's chats search and cite their notes.",
        )}
      </p>
      {kbs.length === 0 ? (
        <p className="rounded-md border border-border/60 px-3 py-2 text-xs text-muted-foreground">
          {t("knowledge.picker.empty", "No knowledge spaces yet.")}
        </p>
      ) : (
        <div className="flex max-h-52 flex-col gap-0.5 overflow-y-auto rounded-md border border-border/60 p-1">
          {kbs.map((kb) => {
            const att = attachmentFor(kb.id)
            const busy = busyId === kb.id
            return (
              <div
                key={kb.id}
                className="flex items-center gap-2 rounded-lg px-1.5 py-1.5 hover:bg-secondary/50"
              >
                <span className="shrink-0 text-sm leading-none">{kb.emoji || "📚"}</span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1">
                    <span className="truncate text-xs font-medium">{kb.name}</span>
                    {kb.external && <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />}
                  </div>
                  <span className="text-[10px] text-muted-foreground">
                    {t("knowledge.picker.noteCount", { count: kb.noteCount })}
                  </span>
                </div>

                {/* Always-visible 关闭/只读/读写 control. External vaults hide the
                    write segment (read-capped, D11). */}
                <KbAccessControl
                  value={!att ? "off" : att.access}
                  allowWrite={!kb.external}
                  busy={busy}
                  onChange={(next) => void setAttach(kb, next === "off" ? null : next)}
                />
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
