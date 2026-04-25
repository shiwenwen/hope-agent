import { useTranslation } from "react-i18next"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

import { OpenClawImportPanel } from "@/components/onboarding/steps/OpenClawImportPanel"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Called after a successful import; refresh the agent list, etc. */
  onImported: () => void
}

/**
 * Full OpenClaw → Hope Agent import in a single dialog.
 *
 * Wraps the same `OpenClawImportPanel` used by the onboarding wizard so the
 * Settings entry stays in lockstep with the first-run flow.
 */
export default function OpenClawImportDialog({
  open,
  onOpenChange,
  onImported,
}: Props) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("onboarding.importOpenClaw.headline")}</DialogTitle>
          <DialogDescription className="whitespace-pre-line">
            {t("onboarding.importOpenClaw.description")}
          </DialogDescription>
        </DialogHeader>
        {open && (
          <OpenClawImportPanel
            hideSkip
            onSkip={() => onOpenChange(false)}
            onImported={() => {
              onImported()
              onOpenChange(false)
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}
