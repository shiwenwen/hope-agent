import { AlertCircle, FilePlus, Import, Library, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { useKnowledgeReembedJobs } from "@/hooks/useKnowledgeReembedJobs"
import { getTransport } from "@/lib/transport-provider"
import type { KnowledgeBaseMeta } from "@/types/knowledge"
import { isLocalModelJobActive, localModelJobPercent } from "@/types/local-model-jobs"

interface KnowledgeEmptyStateProps {
  kb: KnowledgeBaseMeta | null
  readOnly: boolean
  onNewNote: () => void
  onImport: () => void
}

/**
 * Fills the note-list sidebar / center editor area when there's nothing to
 * show a specific note for. Distinguishes four states that used to all be
 * indistinguishable dead air (or the same generic "select a note" message):
 * a bind/reindex scan in progress, a scan that failed, a genuinely-empty
 * space, and "notes exist, none selected yet" — the classic "did binding
 * fail, is the folder empty, or is it still scanning?" ambiguity.
 */
export default function KnowledgeEmptyState({
  kb,
  readOnly,
  onNewNote,
  onImport,
}: KnowledgeEmptyStateProps) {
  const { t } = useTranslation()
  const { jobForKb } = useKnowledgeReembedJobs()
  const job = jobForKb(kb?.id)

  if (job && isLocalModelJobActive(job)) {
    const fileGranular = job.targetKbIds?.length === 1
    const done = Number(job.bytesCompleted ?? 0)
    const total = Number(job.bytesTotal ?? 0)
    const percent = localModelJobPercent(job)
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-muted-foreground">
        <Loader2 className="h-8 w-8 animate-spin opacity-70" />
        <span className="text-sm font-medium text-foreground">
          {t("knowledge.emptyState.scanning", "Scanning this space…")}
        </span>
        <div className="w-full max-w-[220px]">
          <Progress value={percent} indeterminate={percent == null} className="h-1.5" />
        </div>
        {total > 0 && (
          <span className="text-xs">
            {fileGranular
              ? t("knowledge.jobs.progressFiles", {
                  done,
                  total,
                  defaultValue: "{{done}} / {{total}} files",
                })
              : t("settings.knowledgeEmbedding.reembed.progress", { done, total })}
          </span>
        )}
      </div>
    )
  }

  if (job && job.status === "failed") {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-muted-foreground">
        <AlertCircle className="h-8 w-8 text-destructive/80" />
        <span className="text-sm font-medium text-foreground">
          {t("knowledge.emptyState.scanFailed", "Scan failed")}
        </span>
        {job.error && <p className="max-w-[280px] break-words text-xs text-destructive">{job.error}</p>}
        <Button
          variant="outline"
          size="sm"
          className="mt-1"
          onClick={async () => {
            try {
              await getTransport().call("local_model_job_retry", { jobId: job.jobId })
            } catch (e) {
              toast.error(String(e))
            }
          }}
        >
          {t("localModelJobs.actions.retry", "Retry")}
        </Button>
      </div>
    )
  }

  if (kb && kb.noteCount === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-muted-foreground">
        <Library className="h-10 w-10 opacity-40" />
        <span className="text-sm">
          {kb.rootDir
            ? t("knowledge.emptyState.emptyExternal", "No markdown notes found in this folder yet.")
            : t("knowledge.emptyState.emptyInternal", "This space is empty.")}
        </span>
        {!readOnly && (
          <div className="mt-1 flex flex-wrap justify-center gap-2">
            <Button variant="outline" size="sm" onClick={onNewNote}>
              <FilePlus className="mr-1.5 h-3.5 w-3.5" />
              {t("knowledge.newNote", "New note")}
            </Button>
            <Button variant="outline" size="sm" onClick={onImport}>
              <Import className="mr-1.5 h-3.5 w-3.5" />
              {t("knowledge.emptyState.importFiles", "Import files")}
            </Button>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
      <Library className="h-10 w-10 opacity-40" />
      <span className="text-sm">{t("knowledge.emptyEditor", "Select a note to view or edit.")}</span>
    </div>
  )
}
