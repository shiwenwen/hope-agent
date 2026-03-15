import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ShieldAlert, FolderOpen } from "lucide-react"

export interface ApprovalRequest {
  request_id: string
  command: string
  cwd: string
}

interface ApprovalDialogProps {
  requests: ApprovalRequest[]
  onRespond: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => void
}

export default function ApprovalDialog({
  requests,
  onRespond,
}: ApprovalDialogProps) {
  const { t } = useTranslation()

  if (requests.length === 0) return null

  const current = requests[0]
  const total = requests.length

  return (
    <div className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm flex items-center justify-center">
      <div className="bg-card border border-border rounded-2xl shadow-xl max-w-md w-full mx-4 p-6 animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="flex items-center gap-3 mb-4">
          <div className="w-10 h-10 rounded-full bg-amber-500/15 flex items-center justify-center text-amber-500 shrink-0">
            <ShieldAlert className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold text-foreground">
              {t("approval.title")}
            </h3>
            {total > 1 && (
              <span className="text-xs text-muted-foreground">
                {t("approval.queueIndicator", { current: 1, total })}
              </span>
            )}
          </div>
        </div>

        {/* Working Directory */}
        <div className="mb-3">
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
            <FolderOpen className="h-3 w-3" />
            <span>{t("approval.workingDir")}</span>
          </div>
          <div className="text-xs text-foreground/70 font-mono bg-secondary/50 rounded-lg px-2.5 py-1.5 truncate">
            {current.cwd}
          </div>
        </div>

        {/* Command */}
        <div className="mb-5">
          <div className="text-xs text-muted-foreground mb-1">
            {t("approval.command")}
          </div>
          <pre className="text-sm text-foreground font-mono bg-secondary rounded-lg p-3 whitespace-pre-wrap break-all max-h-40 overflow-y-auto leading-relaxed">
            {current.command}
          </pre>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="text-red-400 hover:text-red-300 border-red-500/30 hover:border-red-500/50 hover:bg-red-500/10"
            onClick={() => onRespond(current.request_id, "deny")}
          >
            {t("approval.deny")}
          </Button>
          <div className="flex-1" />
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onRespond(current.request_id, "allow_once")}
          >
            {t("approval.allowOnce")}
          </Button>
          <Button
            size="sm"
            onClick={() => onRespond(current.request_id, "allow_always")}
          >
            {t("approval.allowAlways")}
          </Button>
        </div>
      </div>
    </div>
  )
}
