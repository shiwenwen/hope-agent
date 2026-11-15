import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/switch"
import { NumberInput } from "@/components/ui/number-input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { SessionMeta } from "@/types/chat"
import { researchActivityKey } from "./useAssistantResearch"
import type {
  AssistantPolicySettings,
  AssistantResearchState,
  AssistantSource,
} from "./useAssistantResearch"

export function AssistantResearchSettings({
  state,
  sessions,
}: {
  state: AssistantResearchState
  sessions: SessionMeta[]
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<AssistantPolicySettings | null>(state.policy?.settings ?? null)
  const [dirty, setDirty] = useState(false)
  const [feedback, setFeedback] = useState<"idle" | "saving" | "pending" | "saved" | "failed">(
    "idle",
  )
  const [pendingRevision, setPendingRevision] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [sessionId, setSessionId] = useState("")
  const [relativePath, setRelativePath] = useState("")
  const [focus, setFocus] = useState("")
  const [durationDays, setDurationDays] = useState(7)
  const [sourceBusy, setSourceBusy] = useState<string | null>(null)
  const [sourceReceipt, setSourceReceipt] = useState<string | null>(null)
  const activity = state.activity.find((entry) => entry.id === sourceReceipt)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => () => clearTimeout(timer.current), [])
  useEffect(() => {
    if (!dirty && state.policy) setDraft(state.policy.settings)
    if (pendingRevision === null || !state.policy) return
    if (
      state.policy.requestedRevision > pendingRevision ||
      (state.policy.requestedRevision === pendingRevision && state.policy.failed)
    ) {
      setFeedback("failed")
      setPendingRevision(null)
    } else if (!state.policy.pending && state.policy.appliedRevision === pendingRevision) {
      setFeedback("saved")
      setPendingRevision(null)
      setDirty(false)
      timer.current = setTimeout(() => setFeedback("idle"), 2000)
    }
  }, [state.policy, dirty, pendingRevision])
  const save = async () => {
    if (!draft || !state.policy || feedback === "saving" || feedback === "pending") return
    clearTimeout(timer.current)
    setFeedback("saving")
    setError(null)
    try {
      const receipt = await state.savePolicy(state.policy.requestedRevision, draft)
      setPendingRevision(receipt.requestedRevision)
      setFeedback("pending")
    } catch (cause) {
      setError(String(cause))
      setFeedback("failed")
      timer.current = setTimeout(() => setFeedback("idle"), 2000)
    }
  }
  const share = async () => {
    if (!sessionId || !relativePath.trim() || !focus.trim() || sourceBusy) return
    setSourceBusy("new")
    setError(null)
    try {
      await state.addSource({
        sessionId,
        relativePath: relativePath.trim(),
        focus: focus.trim(),
        durationDays,
      })
      setRelativePath("")
      setFocus("")
    } catch (cause) {
      setError(String(cause))
    } finally {
      setSourceBusy(null)
    }
  }
  const sourceAction = async (id: string, revoke: boolean) => {
    if (sourceBusy) return
    setSourceBusy(id)
    setError(null)
    try {
      if (revoke) await state.revokeSource(id)
      else setSourceReceipt((await state.refreshSource(id)).id)
    } catch (cause) {
      setError(String(cause))
    } finally {
      setSourceBusy(null)
    }
  }
  const disclosure = async (source: AssistantSource, enabled: boolean) => {
    if (sourceBusy) return
    setSourceBusy(source.id)
    setError(null)
    try {
      await state.setSourceDisclosure(source, enabled)
    } catch (cause) {
      setError(String(cause))
    } finally {
      setSourceBusy(null)
    }
  }
  const ownerTime = (seconds: number) => {
    try {
      return new Date(seconds * 1000).toLocaleString(undefined, {
        timeZone: state.policy?.settings.timezone ?? "UTC",
      })
    } catch {
      return new Date(seconds * 1000).toISOString()
    }
  }
  const candidates = sessions.filter(
    (session) =>
      (session.kind ?? "regular") === "regular" &&
      !session.incognito &&
      !session.isCron &&
      !session.archivedAt &&
      !session.parentSessionId &&
      !session.channelInfo &&
      session.origin?.kind !== "codex",
  )
  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">
        {t("personalAssistant.research.settingsHint")}
      </p>
      {draft && (
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault()
            void save()
          }}
        >
          <div className="flex items-center justify-between gap-4">
            <label htmlFor="assistant-research-enabled" className="text-sm">
              {t("personalAssistant.research.enabled")}
            </label>
            <Switch
              id="assistant-research-enabled"
              checked={draft.researchEnabled}
              onCheckedChange={(checked) => {
                setDraft({ ...draft, researchEnabled: checked })
                setDirty(true)
              }}
            />
          </div>
          <div className="flex items-center justify-between gap-4">
            <label htmlFor="assistant-paused" className="text-sm">
              {t("personalAssistant.research.pause")}
            </label>
            <Switch
              id="assistant-paused"
              checked={draft.paused}
              onCheckedChange={(checked) => {
                setDraft({ ...draft, paused: checked })
                setDirty(true)
              }}
            />
          </div>
          <div className="space-y-2">
            <label htmlFor="assistant-notice-route" className="text-sm">
              {t("personalAssistant.noticePolicy.route")}
            </label>
            <Select
              value={draft.noticeRoute}
              onValueChange={(value) => {
                setDraft({ ...draft, noticeRoute: value as AssistantPolicySettings["noticeRoute"] })
                setDirty(true)
              }}
            >
              <SelectTrigger id="assistant-notice-route">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(["none", "desktop", "im"] as const).map((route) => (
                  <SelectItem key={route} value={route}>
                    {t(`personalAssistant.noticePolicy.routes.${route}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {t("personalAssistant.noticePolicy.limitHint")}
            </p>
          </div>
          {draft.noticeRoute !== "none" && (
            <div className="space-y-3">
              <div className="space-y-1">
                <label htmlFor="assistant-owner-timezone" className="text-sm">
                  {t("personalAssistant.noticePolicy.timezone")}
                </label>
                <Input
                  id="assistant-owner-timezone"
                  value={draft.timezone ?? ""}
                  placeholder="Asia/Shanghai"
                  maxLength={128}
                  onChange={(event) => {
                    setDraft({ ...draft, timezone: event.target.value.trim() || null })
                    setDirty(true)
                  }}
                />
              </div>
              <div className="flex flex-wrap items-center gap-3">
                <label htmlFor="assistant-quiet-start" className="text-sm">
                  {t("personalAssistant.noticePolicy.quietStart")}
                </label>
                <NumberInput
                  id="assistant-quiet-start"
                  className="w-20"
                  value={draft.quietStartHour}
                  min={0}
                  max={23}
                  onChange={(event) => {
                    setDraft({
                      ...draft,
                      quietStartHour: Math.max(0, Math.min(23, Number(event.target.value) || 0)),
                    })
                    setDirty(true)
                  }}
                />
                <label htmlFor="assistant-quiet-end" className="text-sm">
                  {t("personalAssistant.noticePolicy.quietEnd")}
                </label>
                <NumberInput
                  id="assistant-quiet-end"
                  className="w-20"
                  value={draft.quietEndHour}
                  min={0}
                  max={23}
                  onChange={(event) => {
                    setDraft({
                      ...draft,
                      quietEndHour: Math.max(0, Math.min(23, Number(event.target.value) || 0)),
                    })
                    setDirty(true)
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("personalAssistant.noticePolicy.quietHint")}
              </p>
            </div>
          )}
          {draft.noticeRoute === "im" && (
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-4">
                <label htmlFor="assistant-desktop-to-im" className="text-sm">
                  {t("personalAssistant.noticePolicy.desktopToIm")}
                </label>
                <Switch
                  id="assistant-desktop-to-im"
                  checked={draft.desktopToImNotices ?? false}
                  onCheckedChange={(checked) => {
                    setDraft({ ...draft, desktopToImNotices: checked })
                    setDirty(true)
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("personalAssistant.noticePolicy.desktopToImHint")}
              </p>
              {!state.attention?.paired && (
                <p role="status" className="text-xs text-muted-foreground">
                  {t("personalAssistant.noticePolicy.pairRequired")}
                </p>
              )}
            </div>
          )}
          <div className="flex items-center justify-end gap-3">
            {feedback === "pending" && (
              <p role="status" className="text-xs text-muted-foreground">
                {t("personalAssistant.research.pendingPolicy")}
              </p>
            )}
            <Button
              type="submit"
              size="sm"
              disabled={!dirty || feedback === "saving" || feedback === "pending"}
              className={
                feedback === "saved"
                  ? "text-green-600"
                  : feedback === "failed"
                    ? "text-destructive"
                    : undefined
              }
            >
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
        </form>
      )}
      {state.attention && (
        <div className="space-y-2 text-xs text-muted-foreground">
          {state.attention.deferredReason && (
            <p role="status">
              {t(`personalAssistant.noticePolicy.deferred.${state.attention.deferredReason}`)}
            </p>
          )}
          {state.attention.nextEligibleAt && state.policy?.settings.noticeRoute !== "none" && (
            <p>
              {t("personalAssistant.noticePolicy.nextEligible", {
                time: ownerTime(state.attention.nextEligibleAt),
                zone: state.policy?.settings.timezone ?? "UTC",
              })}
            </p>
          )}
          {state.attention.receipts.slice(0, 3).map((receipt) => (
            <p key={receipt.id}>
              {t(`personalAssistant.noticePolicy.receipts.${receipt.state}`, {
                time: ownerTime(receipt.submittedAt),
              })}
            </p>
          ))}
        </div>
      )}
      <div className="space-y-3">
        <p className="text-sm font-medium">{t("personalAssistant.research.sources")}</p>
        {state.sources
          .filter((source) => source.state === "active")
          .map((source) => (
            <div key={source.id} className="space-y-2 rounded-md bg-muted/30 p-3">
              <p className="break-all text-sm font-medium">{source.relativePath}</p>
              <p className="whitespace-pre-wrap text-sm text-muted-foreground">{source.focus}</p>
              <p className="text-xs text-muted-foreground">
                {t("personalAssistant.research.expires", {
                  time: new Date(source.expiresAt * 1000).toLocaleString(),
                })}
              </p>
              {source.lastObservedAt && (
                <p className="text-xs text-muted-foreground">
                  {t("personalAssistant.research.observed", {
                    time: new Date(source.lastObservedAt * 1000).toLocaleString(),
                  })}
                </p>
              )}
              {source.suspended && (
                <p className="text-xs text-muted-foreground">
                  {t("personalAssistant.research.suspended")}
                </p>
              )}
              {source.error && (
                <p role="status" className="text-xs text-muted-foreground">
                  {t("personalAssistant.research.observationGap")}
                </p>
              )}
              <div className="flex items-center justify-between gap-4">
                <label htmlFor={`assistant-source-im-${source.id}`} className="text-xs">
                  {t("personalAssistant.noticePolicy.sourceDisclosure")}
                </label>
                <Switch
                  id={`assistant-source-im-${source.id}`}
                  disabled={!!sourceBusy || !state.attention?.pairing}
                  checked={source.imSummaryAllowed ?? false}
                  onCheckedChange={(checked) => void disclosure(source, checked)}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("personalAssistant.noticePolicy.sourceDisclosureHint")}
              </p>
              <div className="flex gap-2">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={!!sourceBusy}
                  onClick={() => void sourceAction(source.id, false)}
                >
                  {t("personalAssistant.research.refresh")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={!!sourceBusy}
                  onClick={() => void sourceAction(source.id, true)}
                >
                  {t("personalAssistant.research.revoke")}
                </Button>
              </div>
            </div>
          ))}
        {!state.sources.some((source) => source.state === "active") && (
          <p className="text-sm text-muted-foreground">
            {t("personalAssistant.research.noSources")}
          </p>
        )}
        <form
          className="space-y-3"
          onSubmit={(event) => {
            event.preventDefault()
            void share()
          }}
        >
          <Select value={sessionId} onValueChange={setSessionId}>
            <SelectTrigger aria-label={t("personalAssistant.research.workspace")}>
              <SelectValue placeholder={t("personalAssistant.research.workspace")} />
            </SelectTrigger>
            <SelectContent>
              {candidates.map((session) => (
                <SelectItem key={session.id} value={session.id}>
                  {session.title || session.id.slice(0, 8)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            value={relativePath}
            maxLength={4096}
            aria-label={t("personalAssistant.research.path")}
            placeholder={t("personalAssistant.research.path")}
            onChange={(event) => setRelativePath(event.target.value)}
          />
          <Textarea
            value={focus}
            maxLength={2000}
            aria-label={t("personalAssistant.research.focus")}
            placeholder={t("personalAssistant.research.focusPlaceholder")}
            onChange={(event) => setFocus(event.target.value)}
          />
          <div className="flex items-center gap-3">
            <label htmlFor="assistant-source-days" className="text-xs text-muted-foreground">
              {t("personalAssistant.research.days")}
            </label>
            <NumberInput
              id="assistant-source-days"
              value={durationDays}
              min={1}
              max={30}
              onChange={(event) =>
                setDurationDays(Math.max(1, Math.min(30, Number(event.target.value) || 7)))
              }
              className="w-24"
            />
          </div>
          <p className="text-xs text-muted-foreground">
            {t("personalAssistant.research.baselineHint")}
          </p>
          <div className="flex justify-end">
            <Button
              type="submit"
              size="sm"
              disabled={!sessionId || !relativePath.trim() || !focus.trim() || !!sourceBusy}
            >
              {t("personalAssistant.research.share")}
            </Button>
          </div>
        </form>
      </div>
      {activity && (
        <p role="status" className="text-sm text-muted-foreground">
          {t(researchActivityKey(activity))}
        </p>
      )}
      {(error || state.error) && (
        <p role="alert" className="text-sm text-destructive">
          {error || state.error}
        </p>
      )}
    </div>
  )
}
