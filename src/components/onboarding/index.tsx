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
import { ONBOARDING_STEPS, type OnboardingDraft, type OnboardingStepKey } from "./types"
import { useOnboarding } from "./useOnboarding"
import { ChannelsStep } from "./steps/ChannelsStep"
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
  /** Lets user pop out to Settings → Channels from Step 8. */
  onOpenSettings: () => void
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
  onOpenSettings,
  onCodexAuth,
  initialLanguage,
}: OnboardingWizardProps) {
  const { t } = useTranslation()
  const onboarding = useOnboarding({ onComplete })
  const {
    step,
    stepKey,
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
        case "server":
          await t.call("apply_onboarding_server", {
            bindAddr:
              draft.server?.bindMode === "lan" ? "0.0.0.0:8420" : "127.0.0.1:8420",
            apiKey: draft.server?.apiKeyEnabled ? draft.server?.apiKey ?? "" : "",
          })
          return true
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
    return true
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
            onLanguageChange={(lang) => patchDraft({ language: lang })}
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
        return <ChannelsStep onOpenSettings={onOpenSettings} />
      case "summary":
        return <SummaryStep draft={draft} skipped={skipped} />
    }
  }

  const isFinal = stepKey === "summary"
  const isProvider = stepKey === "provider"
  const canGoBack = step > 0 && !isFinal
  const canSkip = !isFinal

  async function handleExitConfirm() {
    setExitOpen(false)
    await onboarding.exitAndSave()
    onComplete()
  }

  return (
    <div className="flex items-center justify-center min-h-screen p-4 bg-gradient-to-br from-background to-muted/40">
      <div className="w-full max-w-3xl rounded-xl border border-border bg-card shadow-lg overflow-hidden">
        <div className="flex items-center justify-between px-4 py-2 border-b border-border">
          <div className="text-xs text-muted-foreground">
            {t("onboarding.stepIndicator", {
              current: step + 1,
              total: ONBOARDING_STEPS.length,
            })}
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setExitOpen(true)}
            aria-label={t("onboarding.nav.exit")}
            disabled={busy}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <StepIndicator current={step} skipped={skipped} />

        <div className="min-h-[420px]">{renderStep()}</div>

        <NavigationFooter
          canGoBack={canGoBack}
          canSkip={canSkip && !isProvider ? true : isProvider}
          skipVariant={isProvider ? "danger" : "normal"}
          isFinal={isFinal}
          busy={saving || busy}
          hideNext={isProvider}
          onBack={goBack}
          onSkip={() => void skipCurrent()}
          onNext={() => void handleNext()}
          onFinish={() => void finish()}
          nextLabel={isFinal ? t("onboarding.summary.startButton") : undefined}
        />
      </div>

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
