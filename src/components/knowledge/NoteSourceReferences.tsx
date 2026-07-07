import { AlertTriangle, ExternalLink, FileText, Loader2 } from "lucide-react"
import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { KnowledgeSourceReadResult, NoteSourceRef } from "@/types/knowledge"

interface NoteSourceReferencesProps {
  kbId: string | null
  notePath: string | null
  contentHash?: string | null
}

export default function NoteSourceReferences({
  kbId,
  notePath,
  contentHash,
}: NoteSourceReferencesProps) {
  const { t } = useTranslation()
  const [refs, setRefs] = useState<NoteSourceRef[]>([])
  const [loading, setLoading] = useState(false)
  const [selected, setSelected] = useState<KnowledgeSourceReadResult | null>(null)
  const [readingId, setReadingId] = useState<string | null>(null)

  const load = useCallback(async () => {
    if (!kbId || !notePath) {
      setRefs([])
      return
    }
    setLoading(true)
    try {
      const list = await getTransport().call<NoteSourceRef[]>("kb_note_source_refs_cmd", {
        kbId,
        path: notePath,
      })
      setRefs(list)
    } catch (e) {
      logger.warn("knowledge", "NoteSourceReferences::load", "source refs failed", e)
      setRefs([])
    } finally {
      setLoading(false)
    }
  }, [kbId, notePath])

  useEffect(() => {
    void load()
  }, [load, contentHash])

  useEffect(() => getTransport().listen("knowledge:changed", () => void load()), [load])

  async function openSource(ref: NoteSourceRef) {
    if (!kbId || ref.missing) return
    setReadingId(ref.sourceId)
    try {
      const data = await getTransport().call<KnowledgeSourceReadResult>("kb_source_read_cmd", {
        kbId,
        sourceId: ref.sourceId,
      })
      setSelected(data)
    } catch (e) {
      logger.warn("knowledge", "NoteSourceReferences::openSource", "source read failed", e)
      toast.error(t("knowledge.sources.readFailed", "Couldn't open source"))
    } finally {
      setReadingId(null)
    }
  }

  if (!loading && refs.length === 0) return null

  return (
    <div className="mb-3">
      <div className="mb-1 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        <FileText className="h-3.5 w-3.5" />
        <span>{t("knowledge.sources.evidenceSources", "Sources")}</span>
        {loading ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
      </div>
      <div className="space-y-1">
        {refs.map((ref) => (
          <button
            key={ref.sourceId}
            type="button"
            disabled={ref.missing}
            onClick={() => void openSource(ref)}
            className={cn(
              "flex w-full min-w-0 flex-col gap-0.5 rounded-md border border-border-soft/60 px-2 py-1.5 text-left hover:bg-muted/50",
              ref.missing && "cursor-default opacity-70 hover:bg-transparent",
            )}
          >
            <span className="flex min-w-0 items-center gap-1.5">
              {ref.missing || ref.stale ? (
                <AlertTriangle className="h-3 w-3 shrink-0 text-amber-500" />
              ) : (
                <FileText className="h-3 w-3 shrink-0 text-muted-foreground" />
              )}
              <span className="truncate text-xs font-medium">
                {ref.title || ref.sourceId}
              </span>
              {!ref.missing && readingId === ref.sourceId ? (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />
              ) : !ref.missing ? (
                <ExternalLink className="h-3 w-3 shrink-0 text-muted-foreground" />
              ) : null}
            </span>
            <span className="truncate text-[10px] text-muted-foreground">
              {ref.missing
                ? t("knowledge.sources.missingSource", "Missing source")
                : ref.superseded
                  ? t("knowledge.sources.supersededSource", "Newer source version available")
                  : ref.stale
                  ? t("knowledge.sources.staleSource", "Source changed after it was organized")
                  : ref.originUri || ref.sourceId}
            </span>
          </button>
        ))}
      </div>

      <Dialog open={!!selected} onOpenChange={(open) => !open && setSelected(null)}>
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle className="truncate">{selected?.title}</DialogTitle>
            {selected?.originUri ? (
              <DialogDescription className="truncate">{selected.originUri}</DialogDescription>
            ) : null}
          </DialogHeader>
          <pre className="max-h-[70vh] overflow-auto whitespace-pre-wrap rounded-md border border-border-soft/60 bg-muted/30 p-3 text-xs leading-relaxed">
            {selected?.content}
          </pre>
        </DialogContent>
      </Dialog>
    </div>
  )
}
