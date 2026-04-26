import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ThemeMode } from "@/hooks/useTheme"

import {
  DEFAULT_REMOTE_DRAFT,
  DEFAULT_SERVER_DRAFT,
  stepsForMode,
  type OnboardingDraft,
  type OnboardingStepKey,
} from "./types"
import { CURRENT_ONBOARDING_VERSION } from "./version"

interface UseOnboardingArgs {
  /** Called exactly once after `mark_onboarding_completed` resolves. */
  onComplete: () => void
}

type ProfileDraft = NonNullable<OnboardingDraft["profile"]>
type AiExperience = NonNullable<ProfileDraft["aiExperience"]>
type ResponseStyle = NonNullable<ProfileDraft["responseStyle"]>

const AI_EXPERIENCE_VALUES: readonly AiExperience[] = [
  "beginner",
  "intermediate",
  "expert",
]
const RESPONSE_STYLE_VALUES: readonly ResponseStyle[] = [
  "concise",
  "balanced",
  "detailed",
]

/** Best-effort pre-fill from disk so rerun flows don't ask for values
 *  the user already provided. Individual lookup failures degrade to
 *  empty fields rather than blocking hydration. */
async function seedDraftFromCurrentConfig(): Promise<OnboardingDraft> {
  const t = getTransport()
  const draft: OnboardingDraft = {}

  const [userCfg, theme, serverCfg, approvalAction, skills] = await Promise.all([
    t
      .call<{
        name?: string | null
        timezone?: string | null
        language?: string | null
        aiExperience?: string | null
        responseStyle?: string | null
      }>("get_user_config")
      .catch(() => null),
    t.call<string>("get_theme").catch(() => null),
    t
      .call<{ bindAddr?: string; hasApiKey?: boolean }>("get_server_config")
      .catch(() => null),
    t.call<string>("get_approval_timeout_action").catch(() => null),
    t.call<Array<{ name: string; enabled?: boolean }>>("get_skills").catch(() => []),
  ])

  if (userCfg) {
    if (userCfg.language) draft.language = userCfg.language
    const profile: ProfileDraft = {}
    if (userCfg.name) profile.name = userCfg.name
    if (userCfg.timezone) profile.timezone = userCfg.timezone
    if (
      userCfg.aiExperience &&
      (AI_EXPERIENCE_VALUES as readonly string[]).includes(userCfg.aiExperience)
    ) {
      profile.aiExperience = userCfg.aiExperience as AiExperience
    }
    if (
      userCfg.responseStyle &&
      (RESPONSE_STYLE_VALUES as readonly string[]).includes(userCfg.responseStyle)
    ) {
      profile.responseStyle = userCfg.responseStyle as ResponseStyle
    }
    if (Object.keys(profile).length > 0) draft.profile = profile
  }

  if (theme === "auto" || theme === "light" || theme === "dark") {
    draft.theme = theme as ThemeMode
  }

  if (serverCfg) {
    const addr = String(serverCfg.bindAddr || "")
    draft.server = {
      bindMode: addr.startsWith("0.0.0.0") ? "lan" : "local",
      apiKeyEnabled: Boolean(serverCfg.hasApiKey),
      // API key stays empty — get_server_config returns a masked value.
      // The apply step treats undefined as "preserve existing" so leaving
      // this empty on rerun doesn't regenerate or clear the real key.
      apiKey: "",
    }
  }

  if (approvalAction) {
    // "proceed" is what apply_safety writes when approvals_enabled=false.
    draft.safety = { approvalsEnabled: approvalAction !== "proceed" }
  }

  if (Array.isArray(skills)) {
    const disabledList = skills
      .filter((s) => s.enabled === false)
      .map((s) => s.name)
    if (disabledList.length > 0) draft.skills = { disabled: disabledList }
  }

  return draft
}

function mergeDraft(base: OnboardingDraft, override: OnboardingDraft): OnboardingDraft {
  return {
    ...base,
    ...override,
    profile: { ...base.profile, ...override.profile },
    safety: override.safety ?? base.safety,
    skills: override.skills ?? base.skills,
    server: override.server
      ? { ...(base.server ?? DEFAULT_SERVER_DRAFT), ...override.server }
      : base.server,
    remote: override.remote
      ? { ...(base.remote ?? DEFAULT_REMOTE_DRAFT), ...override.remote }
      : base.remote,
  }
}

interface UseOnboardingReturn {
  step: number
  stepKey: OnboardingStepKey
  /** Active step list for the current mode (full list, or the 2-step remote flow). */
  steps: OnboardingStepKey[]
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

  useEffect(() => {
    if (hydratedRef.current) return
    hydratedRef.current = true
    void (async () => {
      try {
        const state = await getTransport().call<{
          draft?: OnboardingDraft | null
          draftStep?: number
          skippedSteps?: string[]
          everCompleted?: boolean
        }>("get_onboarding_state")
        const seeded = await seedDraftFromCurrentConfig()
        const restoredDraft = state.draft ?? {}

        const mergedDraft = mergeDraft(seeded, restoredDraft)
        if (Object.keys(mergedDraft).length > 0) {
          setDraft(mergedDraft)
        }

        if (typeof state.draftStep === "number") {
          const activeSteps = stepsForMode(mergedDraft.serverMode)
          setStep(Math.max(0, Math.min(state.draftStep, activeSteps.length - 1)))
        }
        if (state.skippedSteps?.length) {
          setSkipped(new Set(state.skippedSteps as OnboardingStepKey[]))
        }
      } catch (e) {
        logger.warn("onboarding", "hydrate", "failed to restore wizard state", e)
      }
    })()
  }, [])

  const steps = useMemo(() => stepsForMode(draft.serverMode), [draft.serverMode])
  const stepKey = steps[step] ?? "summary"

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
    setStep((s) => Math.min(s + 1, steps.length - 1))
  }, [steps.length])

  const goBack = useCallback(() => {
    setStep((s) => Math.max(0, s - 1))
  }, [])

  const skipCurrent = useCallback(async () => {
    const key = steps[step]
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
  }, [step, steps, goNext])

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
      steps,
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
      steps,
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
