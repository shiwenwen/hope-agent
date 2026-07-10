import { useState, useRef, useCallback, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { useClickOutside } from "@/hooks/useClickOutside"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { CircleAlert, Library, Loader2, Lock } from "lucide-react"
import { KbAccessControl } from "@/components/knowledge/KbAccessControl"
import { useSessionAttachments } from "@/components/chat/workspace/useSessionAttachments"
import { workspaceKnowledgeErrorDetail } from "@/components/chat/workspace/workspaceKnowledgeFeedback"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { KnowledgeBaseMeta, KbAccess, KbDraftAttachment } from "@/types/knowledge"

interface Props {
  sessionId: string | null
  projectId?: string | null
  /** Incognito sessions get zero KB access (design D10) — disable the control. */
  disabled?: boolean
  /**
   * Draft mode (no `sessionId` yet): parent-held staged attaches. When wired,
   * the picker reads/writes these instead of calling the backend; ChatScreen
   * forwards them on the `chat` command's `kbAttachments` payload, which the
   * backend applies on the auto-create branch. Ignored when a session exists
   * (live mode wins, using attach_session_kb_cmd directly).
   */
  draftAttachments?: KbDraftAttachment[]
  onDraftAttachChange?: (next: KbDraftAttachment[]) => void
  /** "toolbar" (default) = compact icon button in the composer toolbar; "menu"
   *  = full-width labeled row for the composer "+" overflow when space is tight. */
  variant?: "toolbar" | "menu"
}

/**
 * Compact popover for attaching knowledge spaces to the current session
 * (design D10 "currently effective knowledge bases" surface). Placed in the
 * chat input toolbar alongside AwarenessToggle.
 *
 * Attaching grants the assistant access: user `[[note]]` injection and the
 * `note_*` tools both gate on `effective_kb_access`, which reads exactly these
 * attach rows. Without an attach the default is deny — so this is the bridge
 * that makes the knowledge space reachable from chat.
 */
export default function KnowledgePicker({
  sessionId,
  projectId,
  disabled = false,
  draftAttachments = [],
  onDraftAttachChange,
  variant = "toolbar",
}: Props) {
  const { t } = useTranslation()
  // Draft mode iff no live session and the parent wired a draft handler.
  const draftMode = !sessionId && !!onDraftAttachChange
  const [open, setOpen] = useState(false)
  const [kbs, setKbs] = useState<KnowledgeBaseMeta[]>([])
  // Live attachments + invalidation handled by the shared hook (also used by the
  // Workspace knowledge section). `reload` is called after attach/detach and on
  // popover open. Draft mode never has a sessionId, so the hook stays empty and
  // the draft branches below own that path.
  const {
    attachments,
    reload: reloadAttachments,
    loadErrorDetail: attachmentLoadErrorDetail,
  } = useSessionAttachments(sessionId, projectId)
  const [loading, setLoading] = useState(false)
  const [listLoadErrorDetail, setListLoadErrorDetail] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const ref = useRef<HTMLDivElement>(null)

  useClickOutside(
    ref,
    useCallback(() => setOpen(false), []),
  )

  useEffect(() => {
    if (disabled && open) setOpen(false)
  }, [disabled, open])

  // Load all available (non-archived) spaces when the popover opens.
  useEffect(() => {
    if (!open) return
    setLoading(true)
    setListLoadErrorDetail(null)
    getTransport()
      .call<KnowledgeBaseMeta[]>("list_kbs_cmd", { includeArchived: false })
      .then((spaces) => {
        setKbs(spaces)
        setListLoadErrorDetail(null)
      })
      .catch((e) => {
        logger.error("chat", "KnowledgePicker::loadSpaces", "load failed", e)
        setKbs([])
        setListLoadErrorDetail(workspaceKnowledgeErrorDetail(e))
      })
      .finally(() => setLoading(false))
    reloadAttachments()
  }, [open, reloadAttachments])

  // Prune staged drafts whose KB no longer exists / was archived elsewhere, so
  // the badge count and the replayed attach list stay consistent with the rows
  // we can actually render. Guarded on a real diff to avoid a render loop.
  useEffect(() => {
    if (!draftMode || !open || kbs.length === 0) return
    const valid = draftAttachments.filter((a) => kbs.some((k) => k.id === a.kbId))
    if (valid.length !== draftAttachments.length) onDraftAttachChange?.(valid)
  }, [draftMode, open, kbs, draftAttachments, onDraftAttachChange])

  const attachedCount = draftMode ? draftAttachments.length : attachments.length
  const btnDisabled = disabled || (!sessionId && !draftMode)

  // Normalize both live and draft attaches to the same `{ access, via }` shape the
  // row render consumes. Draft rows are always session-scoped (`via: "session"`).
  const attachmentFor = (id: string): { access: KbAccess; via: string } | undefined => {
    if (draftMode) {
      const d = draftAttachments.find((a) => a.kbId === id)
      return d ? { access: d.access, via: "session" } : undefined
    }
    return attachments.find((a) => a.id === id)
  }

  async function setAttach(kb: KnowledgeBaseMeta, access: KbAccess | null) {
    // Draft mode: mutate the parent-held list optimistically, no transport.
    if (draftMode) {
      const rest = draftAttachments.filter((a) => a.kbId !== kb.id)
      onDraftAttachChange!(access === null ? rest : [...rest, { kbId: kb.id, access }])
      return
    }
    if (!sessionId) return
    setBusyId(kb.id)
    try {
      if (access === null) {
        await getTransport().call("detach_session_kb_cmd", { sessionId, kbId: kb.id })
      } else {
        await getTransport().call("attach_session_kb_cmd", { sessionId, kbId: kb.id, access })
      }
      reloadAttachments()
    } catch (e) {
      logger.error("chat", "KnowledgePicker::setAttach", "attach/detach failed", e)
    } finally {
      setBusyId(null)
    }
  }

  const tipLabel = disabled
    ? t("knowledge.picker.incognitoDisabled")
    : !sessionId && !draftMode
      ? t("knowledge.picker.needSession")
      : t("knowledge.picker.title")

  // Shared body for both the floating popover (toolbar) and the inline accordion
  // (menu). The menu variant expands inline inside the "+" dropdown rather than
  // floating, so it never mis-positions in narrow side panels.
  const pickerBody = (
    <>
      <div className="flex items-center justify-between mb-2">
        <span className="text-[11px] text-muted-foreground font-medium">
          {t("knowledge.picker.heading")}
        </span>
        {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
      </div>

      {listLoadErrorDetail && (
        <KnowledgePickerWarning
          title={t("knowledge.picker.loadFailed", "无法加载知识空间")}
          detail={t("knowledge.picker.errorDetail", "详细信息：{{error}}", {
            error: listLoadErrorDetail,
          })}
        />
      )}
      {attachmentLoadErrorDetail && (
        <KnowledgePickerWarning
          title={t("knowledge.picker.attachmentsLoadFailed", "无法读取本会话挂载状态")}
          detail={t("knowledge.picker.errorDetail", "详细信息：{{error}}", {
            error: attachmentLoadErrorDetail,
          })}
        />
      )}

      {kbs.length === 0 && !loading && !listLoadErrorDetail ? (
        <p className="text-xs text-muted-foreground py-3 text-center">
          {t("knowledge.picker.empty")}
        </p>
      ) : (
        <div className="flex flex-col gap-0.5 max-h-[280px] overflow-y-auto -mx-1 px-1">
          {kbs.map((kb) => {
            const att = attachmentFor(kb.id)
            const viaProject = att?.via === "project"
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
                    {viaProject
                      ? t("knowledge.picker.viaProject")
                      : t("knowledge.picker.noteCount", { count: kb.noteCount })}
                  </span>
                </div>

                {/* Always-visible 关闭/只读/读写 segmented control. External
                    vaults hide the write segment (read-capped, D11); project
                    attaches are managed at the project level (rendered read-only). */}
                <KbAccessControl
                  value={!att ? "off" : att.access}
                  allowWrite={!kb.external && !viaProject}
                  disabled={viaProject}
                  busy={busy}
                  onChange={(next) => setAttach(kb, next === "off" ? null : next)}
                />
              </div>
            )
          })}
        </div>
      )}

      <p className="mt-2 text-[10px] text-muted-foreground leading-relaxed">
        {t("knowledge.picker.hint")}
      </p>
    </>
  )

  return (
    <div className={cn("relative", variant === "menu" && "w-full")} ref={ref}>
      {variant === "menu" ? (
        <button
          type="button"
          disabled={btnDisabled}
          onClick={() => setOpen(!open)}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-all duration-150 hover:bg-secondary/60 disabled:pointer-events-none disabled:opacity-50",
            attachedCount > 0 ? "text-blue-500" : "text-foreground/80 hover:text-foreground",
          )}
        >
          <Library className="h-4 w-4 shrink-0" />
          <span className="truncate">{t("knowledge.picker.title")}</span>
          {attachedCount > 0 && <span className="ml-auto tabular-nums text-xs">{attachedCount}</span>}
        </button>
      ) : (
        <IconTip label={tipLabel}>
          <button
            type="button"
            disabled={btnDisabled}
            onClick={() => setOpen(!open)}
            className={cn(
              "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 disabled:cursor-not-allowed disabled:opacity-50",
              attachedCount > 0
                ? "text-blue-500"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Library className="h-4 w-4" />
            {attachedCount > 0 && <span className="tabular-nums">{attachedCount}</span>}
          </button>
        </IconTip>
      )}

      {open &&
        !btnDisabled &&
        (variant === "menu" ? (
          <div className="mt-1 rounded-lg border border-border/50 bg-background/40 p-2 animate-in fade-in-0 slide-in-from-top-1 duration-150">
            {pickerBody}
          </div>
        ) : (
          <div className="absolute bottom-full left-0 mb-2 w-[300px] bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 p-3 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
            {pickerBody}
          </div>
        ))}
    </div>
  )
}

function KnowledgePickerWarning({ title, detail }: { title: string; detail?: string | null }) {
  return (
    <div className="mb-2 flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-800 dark:text-amber-200">
      <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <div className="min-w-0">
        <div className="font-medium">{title}</div>
        {detail && <div className="mt-0.5 break-words opacity-85">{detail}</div>}
      </div>
    </div>
  )
}
