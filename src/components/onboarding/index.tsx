import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { X } from "lucide-react"

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

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

import { NavigationFooter } from "./NavigationFooter"
import { StepIndicator } from "./StepIndicator"
import { type OnboardingDraft, type OnboardingStepKey } from "./types"
import { useOnboarding } from "./useOnboarding"
import { ChannelsStep } from "./steps/ChannelsStep"
import { ModeStep } from "./steps/ModeStep"
import { PersonalityStep } from "./steps/PersonalityStep"
import { ProfileStep } from "./steps/ProfileStep"
import { ProviderStep } from "./steps/ProviderStep"
import { SafetyStep } from "./steps/SafetyStep"
import { ServerStep } from "./steps/ServerStep"
import { SkillsStep } from "./steps/SkillsStep"
import { SummaryStep } from "./steps/SummaryStep"
import { WelcomeStep } from "./steps/WelcomeStep"

interface OnboardingWizardProps {
  /** Called when the user finishes (or exits mid-flow saving draft). */
  onComplete: () => void
  /**
   * Footer "Configure in Settings" shortcut on the channels step. The
   * wizard marks onboarding complete first (so the user doesn't bounce
   * back on next launch), then routes into the full Channels panel.
   * Channel cards themselves open the Add dialog inline and don't use
   * this callback.
   */
  onJumpToChannelsSettings: () => void
  /** Shared Codex OAuth flow (same handler App.tsx passes to ProviderSetup). */
  onCodexAuth: () => Promise<void>
  /** Initial language so Step 1 shows the current selection. */
  initialLanguage: string
}

/**
 * Top-level wizard orchestrator.
 *
 * Wraps each step in a shared Card + StepIndicator + NavigationFooter.
 * Each step's "Next" dispatches a per-step apply command into ha-core —
 * kept here (not inside the step components) so skip-vs-next logic for
 * persistence is in one place and steps stay declarative.
 */
export function OnboardingWizard({
  onComplete,
  onJumpToChannelsSettings,
  onCodexAuth,
  initialLanguage,
}: OnboardingWizardProps) {
  const { t } = useTranslation()
  const onboarding = useOnboarding({ onComplete })
  const {
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
    finish,
    busy,
  } = onboarding
  const [saving, setSaving] = useState(false)
  const [exitOpen, setExitOpen] = useState(false)

  // Keep the draft snapshot in sync so a refresh mid-wizard doesn't lose
  // inputs. Debounced via the `step` change so we don't write on every
  // keystroke inside a step.
  useEffect(() => {
    if (step === 0) return
    void persistDraft()
  }, [step, persistDraft])

  async function applyCurrentStep(): Promise<boolean> {
    const t = getTransport()
    try {
      switch (stepKey) {
        case "welcome":
          if (draft.language)
            await t.call("apply_onboarding_language", { language: draft.language })
          return true
        case "mode":
          return true
        case "provider":
          // Provider persistence happens inside <ProviderSetup /> on save.
          return true
        case "profile":
          await t.call("apply_onboarding_profile", {
            name: draft.profile?.name ?? "",
            timezone: draft.profile?.timezone ?? "",
            aiExperience: draft.profile?.aiExperience ?? "",
            responseStyle: draft.profile?.responseStyle ?? "",
          })
          return true
        case "personality":
          if (draft.personalityPresetId) {
            await t.call("apply_personality_preset_cmd", {
              presetId: draft.personalityPresetId,
            })
          }
          return true
        case "safety":
          await t.call("apply_onboarding_safety", {
            approvalsEnabled: draft.safety?.approvalsEnabled ?? true,
          })
          return true
        case "skills":
          await t.call("apply_onboarding_skills", {
            disabled: draft.skills?.disabled ?? [],
          })
          return true
        case "server": {
          // Decide what to do with the API key:
          // - enabled + user typed/generated one  → write that value
          // - enabled + empty (first run, toggle  → mint a key on the fly
          //   never triggered `toggleApiKey`)
          // - enabled + empty (rerun, masked key → preserve existing
          //   on disk and field starts blank)        by sending `null`
          // - disabled                              → clear ("")
          let apiKey: string | null = ""
          if (draft.server?.apiKeyEnabled) {
            const typed = draft.server?.apiKey ?? ""
            if (typed) {
              apiKey = typed
            } else {
              // Check whether there's already a key on disk. Rerun
              // flow seeds apiKeyEnabled=true with an empty field because
              // `get_server_config` masks the value; in that case we
              // preserve the existing key by sending `null`. First-run
              // fallback: no existing key, so mint one so the persisted
              // state matches the user's intent.
              const existing = await t
                .call<{ hasApiKey?: boolean }>("get_server_config")
                .catch(() => null)
              if (existing?.hasApiKey) {
                apiKey = null
              } else {
                apiKey = await t.call<string>("generate_api_key")
                patchDraft({
                  server: {
                    bindMode: draft.server?.bindMode ?? "local",
                    apiKeyEnabled: true,
                    apiKey,
                  },
                })
              }
            }
          }
          await t.call("apply_onboarding_server", {
            bindAddr:
              draft.server?.bindMode === "lan" ? "0.0.0.0:8420" : "127.0.0.1:8420",
            apiKey,
          })
          return true
        }
        case "channels":
          // No-op: channels persist through the Settings UI when the user
          // clicks a chip. The wizard just "passes through" this step.
          return true
        case "summary":
          return true
      }
    } catch (e) {
      logger.error("onboarding", "applyCurrentStep", `${stepKey} apply failed`, e)
      return false
    }
  }

  async function handleNext() {
    setSaving(true)
    try {
      const ok = await applyCurrentStep()
      if (!ok) return
      goNext()
    } finally {
      setSaving(false)
    }
  }

  function patchProfile(next: OnboardingDraft["profile"]) {
    patchDraft({ profile: next })
  }

  function renderStep() {
    switch (stepKey) {
      case "welcome":
        return (
          <WelcomeStep
            initialLanguage={draft.language ?? initialLanguage}
            initialTheme={draft.theme ?? "auto"}
            onLanguageChange={(lang) => patchDraft({ language: lang })}
            onThemeChange={(theme) => patchDraft({ theme })}
          />
        )
      case "mode":
        return (
          <ModeStep
            mode={draft.serverMode}
            remoteUrl={draft.remote?.url ?? ""}
            remoteApiKey={draft.remote?.apiKey ?? ""}
            onChange={(patch) => {
              const next: Partial<OnboardingDraft> = {}
              if (patch.mode !== undefined) next.serverMode = patch.mode
              if (patch.remoteUrl !== undefined || patch.remoteApiKey !== undefined) {
                next.remote = {
                  url: patch.remoteUrl ?? draft.remote?.url ?? "",
                  apiKey: patch.remoteApiKey ?? draft.remote?.apiKey ?? "",
                }
              }
              patchDraft(next)
            }}
            onRemoteConnected={() => {
              // Remote connected → mark onboarding complete and exit the
              // wizard. No further local steps apply.
              void finish()
            }}
          />
        )
      case "provider":
        return (
          <ProviderStep
            onProviderSaved={() => {
              // ProviderSetup already wrote the provider + active_model.
              goNext()
            }}
            onCodexAuth={onCodexAuth}
          />
        )
      case "profile":
        return <ProfileStep draft={draft.profile} onChange={patchProfile} />
      case "personality":
        return (
          <PersonalityStep
            selected={draft.personalityPresetId ?? ""}
            onSelect={(id) => patchDraft({ personalityPresetId: id })}
          />
        )
      case "safety":
        return (
          <SafetyStep
            approvalsEnabled={draft.safety?.approvalsEnabled ?? true}
            onChange={(enabled) => patchDraft({ safety: { approvalsEnabled: enabled } })}
          />
        )
      case "skills":
        return (
          <SkillsStep
            initialDisabled={draft.skills?.disabled ?? []}
            onChange={(disabled) => patchDraft({ skills: { disabled } })}
          />
        )
      case "server":
        return (
          <ServerStep
            bindMode={draft.server?.bindMode ?? "local"}
            apiKey={draft.server?.apiKey ?? ""}
            apiKeyEnabled={draft.server?.apiKeyEnabled ?? false}
            onChange={(next) => patchDraft({ server: next })}
          />
        )
      case "channels":
        return (
          <ChannelsStep
            onJumpToSettings={async () => {
              try {
                await getTransport().call("mark_onboarding_completed")
              } catch (e) {
                logger.warn(
                  "onboarding",
                  "jumpToChannels",
                  "mark_onboarding_completed failed",
                  e,
                )
              }
              onJumpToChannelsSettings()
            }}
          />
        )
      case "summary":
        return <SummaryStep draft={draft} skipped={skipped} />
    }
  }

  const isFinal = stepKey === "summary"
  const isProvider = stepKey === "provider"
  const isMode = stepKey === "mode"
  const canGoBack = step > 0 && !isFinal
  const canSkip = !isFinal && !isMode
  // Mode step's "Next" is a no-op for remote (inline Connect drives completion)
  // and also blocked until the user picks either option.
  const modeNextDisabled = isMode && (!draft.serverMode || draft.serverMode === "remote")

  async function handleExitConfirm() {
    setExitOpen(false)
    await onboarding.exitAndSave()
    onComplete()
  }

  return (
    <div className="flex flex-col h-screen bg-gradient-to-br from-background to-muted/40">
      <div
        className="relative h-11 flex items-center justify-center px-6 border-b border-border shrink-0"
        data-tauri-drag-region
      >
        <div className="pointer-events-none text-[11px] font-medium tabular-nums tracking-[0.18em] text-muted-foreground/80">
          {t("onboarding.stepIndicator", {
            current: step + 1,
            total: steps.length,
          })}
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setExitOpen(true)}
          aria-label={t("onboarding.nav.exit")}
          disabled={busy}
          className="absolute right-2 top-1/2 -translate-y-1/2"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <StepIndicator current={step} skipped={skipped} steps={steps} />

      <div className="flex-1 overflow-y-auto">
        <div className="min-h-full flex items-center justify-center py-6">
          <div className={`w-full ${isProvider ? "max-w-3xl" : "max-w-2xl"}`}>
            {renderStep()}
          </div>
        </div>
      </div>

      <NavigationFooter
        canGoBack={canGoBack}
        canSkip={canSkip}
        skipVariant={isProvider ? "danger" : "normal"}
        isFinal={isFinal}
        busy={saving || busy}
        nextDisabled={modeNextDisabled}
        hideNext={isProvider}
        onBack={goBack}
        onSkip={() => void skipCurrent()}
        onNext={() => void handleNext()}
        onFinish={() => void finish()}
        nextLabel={isFinal ? t("onboarding.summary.startButton") : undefined}
      />

      <AlertDialog open={exitOpen} onOpenChange={setExitOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("onboarding.exit.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("onboarding.exit.desc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={handleExitConfirm}>
              {t("onboarding.exit.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

export type { OnboardingStepKey }
export default OnboardingWizard
