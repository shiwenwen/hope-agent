import { useEffect, useRef, useState } from "react"
import { Check, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import { useAvatarUpload } from "@/hooks/useAvatarUpload"
import { getTransportRevision, useTransportRevision } from "@/lib/transport-provider"
import type { SessionMeta } from "@/types/chat"
import { PersonalAssistantAvatar } from "./PersonalAssistantAvatar"
import { PersonalAssistantImPanel } from "./PersonalAssistantImPanel"
import type { AssistantProfile, PersonalAssistantState } from "./usePersonalAssistant"

const ASSISTANT_AVATAR_PRESETS = ["sparkles", "bot", "heart"] as const

function AssistantProfileForm({
  profile,
  onSave,
}: {
  profile: AssistantProfile
  onSave: PersonalAssistantState["saveProfile"]
}) {
  const { t } = useTranslation()
  const revision = useTransportRevision()
  const [draft, setDraft] = useState(profile)
  const [feedback, setFeedback] = useState<"idle" | "saving" | "saved" | "failed">("idle")
  const [error, setError] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  const mounted = useRef(false)
  const busy = useRef(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      clearTimeout(timer.current)
    }
  }, [])
  const current = () => mounted.current && revision === getTransportRevision()
  const { cropSrc, handleAvatarPick, handleCropConfirm, handleCropCancel } = useAvatarUpload({
    fileName: () => `assistant-${crypto.randomUUID()}.png`,
    logCategory: "PersonalAssistantSettings",
    onSaved: (avatar) => {
      if (current()) setDraft((value) => ({ ...value, avatar }))
    },
    onError: (cause) => {
      if (current()) setError(String(cause))
    },
  })
  const name = draft.name.trim()
  const validName = name.length > 0 && Array.from(name).length <= 32 && !/\p{Cc}/u.test(name)
  const save = async () => {
    if (busy.current || uploading || !validName) return
    busy.current = true
    clearTimeout(timer.current)
    setFeedback("saving")
    setError(null)
    try {
      const saved = await onSave({ ...draft, name })
      if (!current() || !saved) return
      setDraft(saved)
      setFeedback("saved")
    } catch (cause) {
      if (!current()) return
      setError(String(cause))
      setFeedback("failed")
    } finally {
      busy.current = false
      if (current()) timer.current = setTimeout(() => setFeedback("idle"), 2000)
    }
  }
  const disabled = feedback === "saving" || uploading
  return (
    <form
      className="space-y-5"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <div className="flex flex-wrap items-center gap-4">
        <PersonalAssistantAvatar avatar={draft.avatar} className="h-16 w-16" />
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => void handleAvatarPick()}
          >
            {t("personalAssistant.settings.uploadAvatar")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled || draft.avatar === null}
            onClick={() => setDraft((value) => ({ ...value, avatar: null }))}
          >
            {t("personalAssistant.settings.resetAvatar")}
          </Button>
        </div>
      </div>
      <div className="space-y-2">
        <p className="text-sm font-medium">{t("personalAssistant.settings.presetAvatar")}</p>
        <div className="flex gap-2">
          {ASSISTANT_AVATAR_PRESETS.map((preset) => (
            <Button
              key={preset}
              type="button"
              variant="ghost"
              className="h-auto p-1"
              disabled={disabled}
              aria-label={t(`personalAssistant.settings.presets.${preset}`)}
              aria-pressed={draft.avatar === `preset:${preset}`}
              onClick={() => setDraft((value) => ({ ...value, avatar: `preset:${preset}` }))}
            >
              <PersonalAssistantAvatar avatar={`preset:${preset}`} />
              {draft.avatar === `preset:${preset}` && <Check className="ml-1 h-3 w-3" />}
            </Button>
          ))}
        </div>
      </div>
      <div className="space-y-2">
        <label htmlFor="assistant-name" className="text-sm font-medium">
          {t("personalAssistant.settings.name")}
        </label>
        <Input
          id="assistant-name"
          value={draft.name}
          maxLength={64}
          disabled={disabled}
          aria-describedby="assistant-name-hint"
          onChange={(event) => setDraft((value) => ({ ...value, name: event.target.value }))}
        />
        <p id="assistant-name-hint" className="text-xs text-muted-foreground">
          {t("personalAssistant.settings.nameHint")}
        </p>
      </div>
      <p className="text-sm text-muted-foreground">
        {t("personalAssistant.settings.identityHint")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex justify-end">
        <Button
          type="submit"
          disabled={disabled || !validName}
          className={
            feedback === "saved"
              ? "text-green-600"
              : feedback === "failed"
                ? "text-destructive"
                : undefined
          }
        >
          {(feedback === "saving" || uploading) && (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          )}
          {feedback === "saved" && <Check className="mr-2 h-4 w-4" />}
          {t(
            feedback === "saving"
              ? "common.saving"
              : feedback === "saved"
                ? "common.saved"
                : feedback === "failed"
                  ? "common.saveFailed"
                  : "common.save",
          )}
        </Button>
      </div>
      {cropSrc && (
        <AvatarCropDialog
          open
          imageSrc={cropSrc}
          onCancel={handleCropCancel}
          onConfirm={async (blob) => {
            setUploading(true)
            await handleCropConfirm(blob)
            if (current()) setUploading(false)
          }}
        />
      )}
    </form>
  )
}

export function PersonalAssistantSettings({
  state,
  sessions,
  open,
  visible = true,
  onOpenChange,
}: {
  state: PersonalAssistantState
  sessions: SessionMeta[]
  open: boolean
  visible?: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const revision = useTransportRevision()
  if (!open || !visible || !state.authorized || !state.profile) return null
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(760px,90dvh)] max-w-2xl flex-col overflow-hidden">
        <DialogHeader>
          <DialogTitle>{t("personalAssistant.settings.title")}</DialogTitle>
          <DialogDescription>{t("personalAssistant.settings.description")}</DialogDescription>
        </DialogHeader>
        <Tabs defaultValue="profile" className="flex min-h-0 flex-1 flex-col">
          <TabsList className="shrink-0 self-start">
            <TabsTrigger value="profile">{t("personalAssistant.settings.profile")}</TabsTrigger>
            <TabsTrigger value="im">{t("personalAssistant.settings.im")}</TabsTrigger>
          </TabsList>
          <TabsContent
            value="profile"
            forceMount
            className="min-h-0 overflow-y-auto py-3 data-[state=inactive]:hidden"
          >
            <AssistantProfileForm
              key={revision}
              profile={state.profile}
              onSave={state.saveProfile}
            />
          </TabsContent>
          <TabsContent value="im" className="min-h-0 overflow-y-auto py-3">
            <p className="mb-3 text-sm text-muted-foreground">
              {t("personalAssistant.settings.botIdentityHint")}
            </p>
            <PersonalAssistantImPanel key={revision} sessions={sessions} visible={visible} />
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}
