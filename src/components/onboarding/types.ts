export type OnboardingStepKey =
  | "welcome"
  | "mode"
  | "provider"
  | "search-provider"
  | "profile"
  | "personality"
  | "safety"
  | "skills"
  | "server"
  | "channels"
  | "summary"

/** Full ordered step list for the local-configuration flow. */
export const ONBOARDING_STEPS: OnboardingStepKey[] = [
  "welcome",
  "provider",
  "search-provider",
  "profile",
  "safety",
  "channels",
]

/**
 * Remote connection is now a secondary action on the welcome page. Existing
 * remote-mode drafts therefore only need the welcome step; local setup uses
 * the normal flow and defaults to local mode without an explicit choice.
 */
export function stepsForMode(mode: "local" | "remote" | undefined): OnboardingStepKey[] {
  if (mode === "remote") return ["welcome"]
  return ONBOARDING_STEPS
}

/** Mirrors `ha-core::config::OnboardingState`. */
export interface OnboardingState {
  completedVersion: number
  completedAt?: string | null
  skippedSteps: string[]
  draft?: OnboardingDraft | null
  draftStep: number
}

/**
 * In-progress user input, kept locally until the wizard persists each
 * step. Also the shape we round-trip through `save_onboarding_draft` when
 * the user exits mid-wizard so the next launch can resume.
 */
export interface OnboardingDraft {
  /** Front-end-only layout version for remapping persisted numeric step indexes. */
  flowVersion?: number
  language?: string
  theme?: "auto" | "light" | "dark"
  profile?: {
    name?: string
    timezone?: string
    aiExperience?: "beginner" | "intermediate" | "expert" | ""
    responseStyle?: "concise" | "balanced" | "detailed" | ""
  }
  personalityPresetId?: "default" | "engineer" | "creative" | "companion" | ""
  safety?: { approvalsEnabled: boolean }
  skills?: { disabled: string[] }
  server?: { bindMode: "local" | "lan"; apiKey?: string; apiKeyEnabled: boolean }
  /**
   * Persisted runtime mode. New local setup defaults to "local"; the welcome
   * page's secondary remote action switches this to "remote" on connection.
   */
  serverMode?: "local" | "remote"
  remote?: { url: string; apiKey?: string }
}

export type ServerDraft = NonNullable<OnboardingDraft["server"]>
export type RemoteDraft = NonNullable<OnboardingDraft["remote"]>

/** Canonical defaults used when partially-merging an `OnboardingDraft` whose
 *  `server` / `remote` field is still undefined. Keep aligned with backend. */
export const DEFAULT_SERVER_DRAFT: ServerDraft = {
  bindMode: "local",
  apiKeyEnabled: false,
}
export const DEFAULT_REMOTE_DRAFT: RemoteDraft = { url: "" }

export type PersonalityPresetId = NonNullable<OnboardingDraft["personalityPresetId"]>

export interface StepSummary {
  key: OnboardingStepKey
  /** Label shown in the Summary step ("Language: Simplified Chinese"). */
  label: string
  /** Raw value the user picked. Empty string for "skipped". */
  value: string
  skipped: boolean
}
