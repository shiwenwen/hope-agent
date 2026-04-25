export type OnboardingStepKey =
  | "welcome"
  | "import-openclaw"
  | "mode"
  | "provider"
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
  "import-openclaw",
  "mode",
  "provider",
  "profile",
  "personality",
  "safety",
  "skills",
  "server",
  "channels",
  "summary",
]

/**
 * Step list after the user has chosen a mode. Remote mode short-circuits
 * the wizard — once the user points at a remote server there is nothing
 * local to configure (profile / provider / skills / etc. all live on the
 * server), so we drop straight from the mode step to completion.
 */
export function stepsForMode(mode: "local" | "remote" | undefined): OnboardingStepKey[] {
  if (mode === "remote") return ["welcome", "import-openclaw", "mode"]
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
   * Which mode the user picked on the new Step 2. "local" continues the
   * normal provider / profile / ... flow. "remote" means they connected
   * to another hope-agent server and the wizard will finish early.
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
