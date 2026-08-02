import { useState } from "react"
import { useTranslation } from "react-i18next"
import { RotateCcw } from "lucide-react"

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
import { Button } from "@/components/ui/button"

interface AgentTabResetControlProps {
  sectionLabel: string
  disabled: boolean
  onReset: () => void
}

export default function AgentTabResetControl({
  sectionLabel,
  disabled,
  onReset,
}: AgentTabResetControlProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  return (
    <>
      <div className="mb-4 flex justify-end">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-7 gap-1.5"
          disabled={disabled}
          onClick={() => setOpen(true)}
        >
          <RotateCcw className="h-3.5 w-3.5" />
          {t("settings.resetDefaultsTabAction")}
        </Button>
      </div>

      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.resetDefaultsTitle", { section: sectionLabel })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.agentResetDefaultsDescription", { section: sectionLabel })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                onReset()
                setOpen(false)
              }}
            >
              {t("common.restoreDefaults")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
