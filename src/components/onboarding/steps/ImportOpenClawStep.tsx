import { useTranslation } from "react-i18next"

import {
  OpenClawImportPanel,
  type OpenClawImportSummary,
} from "./OpenClawImportPanel"

interface ImportOpenClawStepProps {
  /**
   * Called after the user finishes (either by skipping or by importing). The
   * wizard advances to the next step after this fires; `summary` is null for
   * skip and the import summary for a successful run.
   */
  onContinue: (summary: OpenClawImportSummary | null) => void
}

/**
 * Step 2 — let users import their existing OpenClaw configuration in one
 * shot. The panel auto-scans on mount; if no OpenClaw state dir is present
 * the panel renders a "not detected" branch and the user just clicks
 * "Continue" to advance.
 */
export function ImportOpenClawStep({ onContinue }: ImportOpenClawStepProps) {
  const { t } = useTranslation()
  return (
    <div className="px-4 sm:px-8 py-6 space-y-4">
      <div className="text-center space-y-2">
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("onboarding.importOpenClaw.headline")}
        </h1>
        <p className="text-sm text-muted-foreground max-w-xl mx-auto whitespace-pre-line">
          {t("onboarding.importOpenClaw.description")}
        </p>
      </div>
      <OpenClawImportPanel
        onSkip={() => onContinue(null)}
        onImported={(summary) => onContinue(summary)}
      />
    </div>
  )
}
