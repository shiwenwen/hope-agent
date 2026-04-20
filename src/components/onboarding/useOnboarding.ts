import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

import { ONBOARDING_STEPS, type OnboardingDraft, type OnboardingStepKey } from "./types"
import { CURRENT_ONBOARDING_VERSION } from "./version"

interface UseOnboardingArgs {
  /** Called exactly once after `mark_onboarding_completed` resolves. */
  onComplete: () => void
}

interface UseOnboardingReturn {
  step: number
  stepKey: OnboardingStepKey
  draft: OnboardingDraft
  skipped: Set<OnboardingStepKey>
  /** Partially merge a draft patch, optimistically (no persistence). */
  patchDraft: (patch: Partial<OnboardingDraft>) => void
  /** Persist the current draft snapshot + step index to the backend. */
  persistDraft: () => Promise<void>
  goNext: () => void
  goBack: () => void
  skipCurrent: () => Promise<void>
  /** Called by the top-right X button to exit mid-wizard. */
  exitAndSave: () => Promise<void>
  /** Final step confirm — writes `mark_onboarding_completed` then fires `onComplete`. */
  finish: () => Promise<void>
  busy: boolean
}

/**
 * Wizard state machine. Hydrates from the server on mount so a resumed
 * launch continues at the previous `draftStep`.
 */
export function useOnboarding({ onComplete }: UseOnboardingArgs): UseOnboardingReturn {
  const [step, setStep] = useState(0)
  const [draft, setDraft] = useState<OnboardingDraft>({})
  const [skipped, setSkipped] = useState<Set<OnboardingStepKey>>(new Set())
  const [busy, setBusy] = useState(false)
  const hydratedRef = useRef(false)

  // Initial hydration — restore draft / step from AppConfig.onboarding.
  useEffect(() => {
    if (hydratedRef.current) return
    hydratedRef.current = true
    void (async () => {
      try {
        const state = await getTransport().call<{
          draft?: OnboardingDraft | null
          draftStep?: number
          skippedSteps?: string[]
        }>("get_onboarding_state")
        if (state.draft) setDraft(state.draft)
        if (typeof state.draftStep === "number") {
          setStep(Math.max(0, Math.min(state.draftStep, ONBOARDING_STEPS.length - 1)))
        }
        if (state.skippedSteps?.length) {
          setSkipped(new Set(state.skippedSteps as OnboardingStepKey[]))
        }
      } catch (e) {
        logger.warn("onboarding", "hydrate", "failed to restore wizard state", e)
      }
    })()
  }, [])

  const stepKey = ONBOARDING_STEPS[step] ?? "summary"

  const patchDraft = useCallback((patch: Partial<OnboardingDraft>) => {
    setDraft((prev) => ({ ...prev, ...patch }))
  }, [])

  const persistDraft = useCallback(async () => {
    try {
      await getTransport().call("save_onboarding_draft", {
        step,
        draft,
      })
    } catch (e) {
      logger.warn("onboarding", "persistDraft", "save_onboarding_draft failed", e)
    }
  }, [draft, step])

  const goNext = useCallback(() => {
    setStep((s) => Math.min(s + 1, ONBOARDING_STEPS.length - 1))
  }, [])

  const goBack = useCallback(() => {
    setStep((s) => Math.max(0, s - 1))
  }, [])

  const skipCurrent = useCallback(async () => {
    const key = ONBOARDING_STEPS[step]
    if (!key) return
    setSkipped((prev) => {
      if (prev.has(key)) return prev
      const next = new Set(prev)
      next.add(key)
      return next
    })
    try {
      await getTransport().call("mark_onboarding_skipped", { stepKey: key })
    } catch (e) {
      logger.warn("onboarding", "skipCurrent", "mark_onboarding_skipped failed", e)
    }
    goNext()
  }, [step, goNext])

  const exitAndSave = useCallback(async () => {
    setBusy(true)
    try {
      await persistDraft()
    } finally {
      setBusy(false)
    }
  }, [persistDraft])

  const finish = useCallback(async () => {
    setBusy(true)
    try {
      await getTransport().call("mark_onboarding_completed")
      onComplete()
    } catch (e) {
      logger.error("onboarding", "finish", "mark_onboarding_completed failed", e)
    } finally {
      setBusy(false)
    }
  }, [onComplete])

  return useMemo(
    () => ({
      step,
      stepKey,
      draft,
      skipped,
      patchDraft,
      persistDraft,
      goNext,
      goBack,
      skipCurrent,
      exitAndSave,
      finish,
      busy,
    }),
    [
      step,
      stepKey,
      draft,
      skipped,
      patchDraft,
      persistDraft,
      goNext,
      goBack,
      skipCurrent,
      exitAndSave,
      finish,
      busy,
    ],
  )
}

export { CURRENT_ONBOARDING_VERSION }
