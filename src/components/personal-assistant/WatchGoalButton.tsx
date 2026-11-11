import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"

export function WatchGoalButton({
  sessionId,
  goalId,
  watching,
  onChange,
}: {
  sessionId: string
  goalId: string
  watching: boolean
  onChange: (sessionId: string, goalId: string, watching: boolean) => Promise<void>
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  return (
    <Button
      size="sm"
      variant="ghost"
      disabled={busy}
      onClick={async () => {
        setBusy(true)
        try {
          await onChange(sessionId, goalId, watching)
        } catch (error) {
          toast.error(String(error))
        } finally {
          setBusy(false)
        }
      }}
    >
      {t(
        busy
          ? "common.loading"
          : watching
            ? "personalAssistant.unwatchGoal"
            : "personalAssistant.watchGoal",
      )}
    </Button>
  )
}
