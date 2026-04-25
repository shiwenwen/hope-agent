import { useTranslation } from "react-i18next"

import openClawLogoDark from "@/assets/openclaw-logo-text-dark.svg"
import openClawLogoLight from "@/assets/openclaw-logo-text-light.svg"

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
      <div className="flex flex-col items-center text-center gap-3">
        <img
          src={openClawLogoDark}
          alt="OpenClaw"
          className="h-12 w-auto block dark:hidden"
          draggable={false}
        />
        <img
          src={openClawLogoLight}
          alt="OpenClaw"
          className="h-12 w-auto hidden dark:block"
          draggable={false}
        />
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
