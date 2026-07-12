import { useTranslation } from "react-i18next"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

import { OpenClawImportPanel } from "./OpenClawImportPanel"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Called after a successful import; refresh the agent list, etc. */
  onImported: () => void
}

/**
 * Full OpenClaw → Hope Agent import in a single dialog.
 *
 * The migration flow intentionally lives in Settings instead of onboarding,
 * keeping third-party imports available without making them part of first run.
 */
export default function OpenClawImportDialog({ open, onOpenChange, onImported }: Props) {
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
