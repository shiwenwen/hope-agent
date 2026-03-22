import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/button"
import { AlertTriangle, ChevronDown, ChevronUp, X } from "lucide-react"
import { cn } from "@/lib/utils"

interface DiagnosisResult {
  cause: string
  severity: string
  user_actionable: boolean
  recommendations: string[]
  auto_fix_applied: string[]
  provider_used: string | null
}

interface RecoveryInfo {
  recovered: boolean
  crashCount: number
  diagnosis?: DiagnosisResult
}

export default function CrashRecoveryBanner() {
  const { t } = useTranslation()
  const [info, setInfo] = useState<RecoveryInfo | null>(null)
  const [dismissed, setDismissed] = useState(false)
  const [expanded, setExpanded] = useState(false)

  useEffect(() => {
    invoke<RecoveryInfo>("get_crash_recovery_info")
      .then((data) => {
        if (data.recovered) {
          setInfo(data)
        }
      })
      .catch(() => {})
  }, [])

  if (!info || dismissed) return null

  return (
    <div className="mx-4 mt-2 rounded-lg border border-yellow-500/30 bg-yellow-500/5 p-3">
      <div className="flex items-start gap-2">
        <AlertTriangle className="h-4 w-4 shrink-0 text-yellow-500 mt-0.5" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-yellow-500">
              {t("health.recoveryBannerTitle")}
            </span>
            {info.diagnosis && (
              <button
                className="text-xs text-muted-foreground hover:text-foreground"
                onClick={() => setExpanded(!expanded)}
              >
                {expanded ? (
                  <ChevronUp className="h-3.5 w-3.5" />
                ) : (
                  <ChevronDown className="h-3.5 w-3.5" />
                )}
              </button>
            )}
          </div>
          <p className="text-xs text-muted-foreground mt-0.5">
            {t("health.recoveryBannerDesc", { count: info.crashCount })}
          </p>

          {expanded && info.diagnosis && (
            <div className="mt-2 space-y-1.5 text-xs">
              <div>
                <span className="text-muted-foreground">{t("health.cause")}: </span>
                <span>{info.diagnosis.cause}</span>
              </div>
              {info.diagnosis.recommendations.length > 0 && (
                <ul className="list-disc list-inside space-y-0.5 text-muted-foreground">
                  {info.diagnosis.recommendations.map((rec, i) => (
                    <li key={i}>{rec}</li>
                  ))}
                </ul>
              )}
              {info.diagnosis.auto_fix_applied.length > 0 && (
                <div className="text-green-500">
                  {t("health.autoFixesApplied")}: {info.diagnosis.auto_fix_applied.join(", ")}
                </div>
              )}
            </div>
          )}
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 w-6 p-0 shrink-0"
          onClick={() => setDismissed(true)}
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  )
}
