import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { requestNeedsResponse } from "./usePersonalAssistant"
import { PersonalAssistantImPanel } from "./PersonalAssistantImPanel"
import type { SessionMeta } from "@/types/chat"
import type {
  AssistantRequest,
  AssistantWatch,
  PersonalAssistantState,
} from "./usePersonalAssistant"

export function PersonalAssistantPanel({
  state,
  onOpenRequest,
  onOpenGoal,
  sessions = [],
  visible = true,
}: {
  state: PersonalAssistantState
  onOpenRequest: (request: AssistantRequest) => Promise<void>
  onOpenGoal: (watch: AssistantWatch) => Promise<void>
  sessions?: SessionMeta[]
  visible?: boolean
}) {
  const { t } = useTranslation()
  const [pending, setPending] = useState<string | null>(null)
  const act = async (id: string, action: () => Promise<void>) => {
    setPending(id)
    try {
      await action()
    } catch (error) {
      toast.error(String(error))
    } finally {
      setPending(null)
    }
  }
  return (
    <section
      aria-label={t("personalAssistant.title")}
      className="min-w-0 border-b border-border-soft px-4 py-2"
    >
      <Tabs defaultValue="work">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">{t("personalAssistant.title")}</span>
          <TabsList className="h-auto max-w-full flex-wrap justify-start">
            <TabsTrigger value="work">{t("personalAssistant.work")}</TabsTrigger>
            <TabsTrigger value="goals">{t("personalAssistant.goals")}</TabsTrigger>
            {state.authorized && (
              <TabsTrigger value="im">
                {t("personalAssistant.im.title", "Connect Telegram")}
              </TabsTrigger>
            )}
          </TabsList>
          <Button
            variant="ghost"
            size="sm"
            disabled={state.loading}
            onClick={() => void state.refresh()}
          >
            {t(state.loading ? "common.loading" : "common.refresh")}
          </Button>
        </div>
        <p className="py-1 text-xs text-muted-foreground">
          {t("personalAssistant.historyLocked")}
        </p>
        {state.error && (
          <p role="alert" className="py-2 text-sm text-destructive">
            {state.error}
          </p>
        )}
        <TabsContent value="work" className="max-h-56 space-y-2 overflow-y-auto">
          {!state.error && !state.loading && state.requests.length === 0 && (
            <p className="text-sm text-muted-foreground">{t("personalAssistant.emptyWork")}</p>
          )}
          {state.requests.map((request) => (
            <article key={request.id} className="rounded-md bg-muted/40 px-3 py-2 text-sm">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong className="min-w-0 truncate">
                  {request.targetTitle || t("personalAssistant.source")}
                </strong>
                <span role="status" className="text-muted-foreground">
                  {t(`personalAssistant.status.${request.status}`)}
                </span>
              </div>
              <p className="whitespace-pre-wrap break-words">{request.instruction}</p>
              {request.resultPreview && (
                <blockquote className="mt-2 whitespace-pre-wrap break-words border-l-2 border-border pl-3 text-muted-foreground">
                  {request.resultPreview}
                </blockquote>
              )}
              {requestNeedsResponse(request) && (
                <p role="status" className="text-amber-700 dark:text-amber-400">
                  {t("chat.pendingInteractionHint")}
                </p>
              )}
              {request.error && <p className="text-destructive">{request.error}</p>}
              <div className="mt-1 flex gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={pending !== null}
                  onClick={() => void act(request.id, () => onOpenRequest(request))}
                >
                  {t("personalAssistant.source")}
                </Button>
                {(request.status === "queued" || request.status === "running") && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={pending !== null}
                    onClick={() => void act(request.id, () => state.cancel(request.id))}
                  >
                    {t("personalAssistant.stop")}
                  </Button>
                )}
              </div>
            </article>
          ))}
        </TabsContent>
        <TabsContent value="goals" className="max-h-56 space-y-2 overflow-y-auto">
          {!state.error && !state.loading && state.watches.length === 0 && (
            <p className="text-sm text-muted-foreground">{t("personalAssistant.emptyGoals")}</p>
          )}
          {state.watches.map((watch) => (
            <article key={watch.goalId} className="rounded-md bg-muted/40 px-3 py-2 text-sm">
              <p className="break-words">{watch.objective}</p>
              {watch.needsAttention && watch.reason && (
                <p role="status" className="text-amber-700 dark:text-amber-400">
                  {t(`personalAssistant.${watch.reason}`)}
                </p>
              )}
              <div className="mt-1 flex gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={pending !== null}
                  onClick={() => void act(watch.goalId, () => onOpenGoal(watch))}
                >
                  {t("personalAssistant.source")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={pending !== null}
                  onClick={() =>
                    void act(watch.goalId, () =>
                      state.watchGoal(watch.sessionId, watch.goalId, true),
                    )
                  }
                >
                  {t("personalAssistant.unwatchGoal")}
                </Button>
              </div>
            </article>
          ))}
        </TabsContent>
        {state.authorized && (
          <TabsContent value="im" className="max-h-[60vh] overflow-y-auto">
            <PersonalAssistantImPanel sessions={sessions} visible={visible} />
          </TabsContent>
        )}
      </Tabs>
    </section>
  )
}
