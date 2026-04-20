import { useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
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

interface NavigationFooterProps {
  canGoBack: boolean
  canSkip: boolean
  /** "danger" = destructive red skip button (Provider step). */
  skipVariant?: "normal" | "danger"
  /** If true, hide the next button and show a finish primary instead. */
  isFinal?: boolean
  /** Disabled Next/Finish button (e.g. when async save is in flight). */
  busy?: boolean
  /** Optional override for the Next button label (e.g. "Get Started"). */
  nextLabel?: string
  /** Show a hidden Next button when the step fully owns its own CTA (e.g.
   *  Step 2 Provider uses the existing ProviderSetup save button). */
  hideNext?: boolean
  onBack: () => void
  onSkip: () => void
  onNext: () => void
  onFinish?: () => void
}

/**
 * Bottom action row for the wizard card.
 *
 * Skip is always the secondary action. For the Provider step the caller
 * passes `skipVariant="danger"` which turns the Skip button red and
 * routes clicks through an AlertDialog confirmation so the user has one
 * last chance to stay on the only step that actually blocks chatting.
 */
export function NavigationFooter({
  canGoBack,
  canSkip,
  skipVariant = "normal",
  isFinal = false,
  busy = false,
  nextLabel,
  hideNext = false,
  onBack,
  onSkip,
  onNext,
  onFinish,
}: NavigationFooterProps) {
  const { t } = useTranslation()
  const [confirmOpen, setConfirmOpen] = useState(false)

  function triggerSkip() {
    if (skipVariant === "danger") setConfirmOpen(true)
    else onSkip()
  }

  return (
    <div className="flex items-center justify-between gap-2 border-t border-border px-6 py-4">
      <div>
        {canGoBack && (
          <Button variant="ghost" size="sm" onClick={onBack} disabled={busy}>
            {t("onboarding.nav.back")}
          </Button>
        )}
      </div>

      <div className="flex items-center gap-2">
        {canSkip && (
          <Button
            variant={skipVariant === "danger" ? "destructive" : "outline"}
            size="sm"
            onClick={triggerSkip}
            disabled={busy}
          >
            {t("onboarding.nav.skip")}
          </Button>
        )}
        {!hideNext && !isFinal && (
          <Button size="sm" onClick={onNext} disabled={busy}>
            {nextLabel ?? t("onboarding.nav.next")}
          </Button>
        )}
        {isFinal && onFinish && (
          <Button size="sm" onClick={onFinish} disabled={busy}>
            {nextLabel ?? t("onboarding.nav.finish")}
          </Button>
        )}
      </div>

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("onboarding.nav.skipConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("onboarding.nav.skipDanger")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setConfirmOpen(false)
                onSkip()
              }}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("onboarding.nav.skipAnyway")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
