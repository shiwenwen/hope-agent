export type OnboardingStepKey =
  | "welcome"
  | "provider"
  | "profile"
  | "personality"
  | "safety"
  | "skills"
  | "server"
  | "channels"
  | "summary"

export const ONBOARDING_STEPS: OnboardingStepKey[] = [
  "welcome",
  "provider",
  "profile",
  "personality",
  "safety",
  "skills",
  "server",
  "channels",
  "summary",
]

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
  language?: string
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
}

export type PersonalityPresetId = NonNullable<OnboardingDraft["personalityPresetId"]>

export interface StepSummary {
  key: OnboardingStepKey
  /** Label shown in the Summary step ("Language: Simplified Chinese"). */
  label: string
  /** Raw value the user picked. Empty string for "skipped". */
  value: string
  skipped: boolean
}
