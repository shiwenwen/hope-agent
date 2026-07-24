import { useCallback, useMemo, useState, type KeyboardEvent } from "react"
import { Check, ChevronLeft, ChevronRight, HelpCircle, Send, Star, Timer } from "lucide-react"
import { useTranslation } from "react-i18next"
import type {
  AskUserLocalizedText,
  AskUserQuestion,
  AskUserQuestionAnswer,
  AskUserQuestionGroup,
} from "@/components/chat/ask-user/AskUserQuestionBlock"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { formatRemaining, useCountdownRemainingSec } from "@/lib/countdown"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

interface PetAskUserCardProps {
  group: AskUserQuestionGroup
  onSubmitted?: () => void
  queuePosition?: { current: number; total: number }
  measuring?: boolean
}

interface QuestionState {
  selected: Set<string>
  customSelected: boolean
  customInput: string
}

function localizedText(
  value: AskUserLocalizedText | undefined | null,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!value) return ""
  if (typeof value === "string") return value
  return t(value.key, {
    ...(value.params ?? {}),
    defaultValue: value.fallback || value.key,
  })
}

function initialAnswers(group: AskUserQuestionGroup): Record<string, QuestionState> {
  return Object.fromEntries(
    group.questions.map((question) => [
      question.questionId,
      { selected: new Set<string>(), customSelected: false, customInput: "" },
    ]),
  )
}

function isFreeText(question: AskUserQuestion): boolean {
  return question.inputKind === "text" || question.inputKind === "textarea"
}

function hasAnswer(question: AskUserQuestion, state: QuestionState | undefined): boolean {
  if (!state) return false
  if (isFreeText(question)) return !!state.customInput.trim()
  if (state.customSelected && !state.customInput.trim()) return false
  return state.selected.size > 0 || (state.customSelected && !!state.customInput.trim())
}

export function PetAskUserCard({
  group,
  onSubmitted,
  queuePosition,
  measuring,
}: PetAskUserCardProps) {
  const { t } = useTranslation()
  const [questionIndex, setQuestionIndex] = useState(0)
  const [answers, setAnswers] = useState<Record<string, QuestionState>>(() => initialAnswers(group))
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const question = group.questions[questionIndex]
  const state = question ? answers[question.questionId] : undefined
  const remaining = useCountdownRemainingSec(
    group.localTimeoutAtMs ?? (group.timeoutAt ? group.timeoutAt * 1000 : null),
  )
  const timedOut = remaining !== null && remaining <= 0
  const lowTime = remaining !== null && remaining > 0 && remaining <= 10
  const isLast = questionIndex >= group.questions.length - 1
  const canContinue = !!question && hasAnswer(question, state) && !timedOut
  const isEnterPlanModeAsk =
    group.questions.length === 1 && group.questions[0]?.questionId === "enter_plan_mode"
  const context = useMemo(() => {
    if (!group.context) return ""
    const raw = localizedText(group.context, t)
    if (!isEnterPlanModeAsk) return raw
    return raw
      ? t("planMode.enterDialog.contextPrefix", { reason: raw })
      : t("planMode.enterDialog.contextNoReason")
  }, [group.context, isEnterPlanModeAsk, t])

  const toggleOption = useCallback((question: AskUserQuestion, value: string) => {
    setError(null)
    setAnswers((current) => {
      const existing = current[question.questionId]
      if (!existing) return current
      const selected = new Set(existing.selected)
      if (question.multiSelect) {
        if (selected.has(value)) selected.delete(value)
        else selected.add(value)
      } else {
        selected.clear()
        selected.add(value)
      }
      return {
        ...current,
        [question.questionId]: {
          ...existing,
          selected,
          customSelected: question.multiSelect ? existing.customSelected : false,
        },
      }
    })
  }, [])

  const toggleCustom = useCallback((question: AskUserQuestion) => {
    setError(null)
    setAnswers((current) => {
      const existing = current[question.questionId]
      if (!existing) return current
      return {
        ...current,
        [question.questionId]: {
          ...existing,
          selected: question.multiSelect ? new Set(existing.selected) : new Set<string>(),
          customSelected: question.multiSelect ? !existing.customSelected : true,
        },
      }
    })
  }, [])

  const updateCustomInput = useCallback((questionId: string, value: string) => {
    setError(null)
    setAnswers((current) => {
      const existing = current[questionId]
      if (!existing) return current
      return { ...current, [questionId]: { ...existing, customInput: value } }
    })
  }, [])

  const submit = useCallback(async () => {
    const missingIndex = group.questions.findIndex(
      (candidate) => !hasAnswer(candidate, answers[candidate.questionId]),
    )
    if (missingIndex >= 0) {
      setQuestionIndex(missingIndex)
      setError(
        t("planMode.question.customRequired", {
          defaultValue: "Answer this question before continuing.",
        }),
      )
      return
    }
    setSubmitting(true)
    setError(null)
    try {
      const response: AskUserQuestionAnswer[] = group.questions.map((candidate) => {
        const answer = answers[candidate.questionId]
        const customInput =
          isFreeText(candidate) || answer.customSelected ? answer.customInput.trim() : ""
        return {
          questionId: candidate.questionId,
          selected: Array.from(answer.selected),
          customInput: customInput || undefined,
        }
      })
      await getTransport().call("respond_ask_user_question", {
        requestId: group.requestId,
        answers: response,
      })
      onSubmitted?.()
    } catch {
      setError(t("common.error", { defaultValue: "Could not submit your answers" }))
      logger.warn(
        "pet",
        "PetAskUserCard::submit",
        "Failed to submit a Pet ask-user response",
        { errorKind: "submit_failed" },
        group.sessionId,
      )
    } finally {
      setSubmitting(false)
    }
  }, [answers, group, onSubmitted, t])

  if (!question) return null
  const questionText = isEnterPlanModeAsk
    ? t("planMode.enterDialog.question")
    : localizedText(question.text, t)
  const next = () => {
    if (!canContinue) return
    setError(null)
    if (isLast) void submit()
    else setQuestionIndex((current) => Math.min(group.questions.length - 1, current + 1))
  }
  const onTextKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return
    event.preventDefault()
    next()
  }

  return (
    <section
      aria-label={t("planMode.question.title", { defaultValue: "AI needs your input" })}
      aria-hidden={measuring || undefined}
      className="w-[360px] rounded-[18px] border border-border/80 bg-popover/95 p-2.5 text-popover-foreground shadow-lg backdrop-blur-md"
    >
      <header className="flex min-h-5 items-center gap-1.5 text-xs">
        <HelpCircle className="h-3.5 w-3.5 shrink-0 text-sky-600 dark:text-sky-400" />
        <h2 className="truncate font-semibold">
          {t("planMode.question.title", { defaultValue: "AI needs your input" })}
        </h2>
        <span className="shrink-0 text-[10px] text-muted-foreground">
          {questionIndex + 1}/{group.questions.length}
          {question.multiSelect
            ? ` · ${t("planMode.question.multiSelect", { defaultValue: "multi" })}`
            : ""}
          {queuePosition && queuePosition.total > 1
            ? ` · ${queuePosition.current}/${queuePosition.total}`
            : ""}
        </span>
        {remaining !== null && (
          <span
            className={cn(
              "ml-auto flex shrink-0 items-center gap-1 text-[10px] tabular-nums",
              timedOut ? "text-destructive" : lowTime ? "text-amber-600" : "text-muted-foreground",
            )}
          >
            <Timer className="h-3 w-3" />
            {timedOut
              ? t("planMode.question.timedOut", { defaultValue: "timed out" })
              : formatRemaining(remaining)}
          </span>
        )}
      </header>

      {context && <p className="mt-1 line-clamp-1 text-[11px] text-muted-foreground">{context}</p>}

      <div className="mt-2">
        <div className="flex items-start gap-2">
          <p className="min-w-0 flex-1 text-sm font-medium leading-[18px]">{questionText}</p>
          {(isEnterPlanModeAsk || question.header) && (
            <span className="max-w-20 shrink-0 truncate rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {isEnterPlanModeAsk
                ? t("planMode.enterDialog.header")
                : localizedText(question.header, t)}
            </span>
          )}
        </div>

        {isFreeText(question) ? (
          question.inputKind === "textarea" ? (
            <Textarea
              autoFocus={!measuring}
              surface="embedded"
              rows={2}
              disabled={submitting || timedOut || measuring}
              value={state?.customInput ?? ""}
              onChange={(event) => updateCustomInput(question.questionId, event.target.value)}
              placeholder={questionText}
              className="mt-1.5 min-h-14 resize-none rounded-lg text-sm"
            />
          ) : (
            <Input
              autoFocus={!measuring}
              surface="embedded"
              disabled={submitting || timedOut || measuring}
              value={state?.customInput ?? ""}
              onChange={(event) => updateCustomInput(question.questionId, event.target.value)}
              onKeyDown={onTextKeyDown}
              placeholder={questionText}
              className="mt-1.5 h-8 rounded-lg text-sm"
            />
          )
        ) : (
          <div className="mt-1.5 max-h-36 space-y-1 overflow-y-auto pr-1">
            {question.options.map((option) => {
              const selected = state?.selected.has(option.value) ?? false
              return (
                <Button
                  key={option.value}
                  type="button"
                  variant="ghost"
                  disabled={submitting || timedOut || measuring}
                  aria-pressed={selected}
                  onClick={() => toggleOption(question, option.value)}
                  className={cn(
                    "h-auto min-h-8 w-full justify-start gap-1.5 rounded-lg px-2 py-1.5 text-left hover:bg-muted/70",
                    selected && "bg-muted",
                  )}
                >
                  <span
                    className={cn(
                      "flex h-3.5 w-3.5 shrink-0 items-center justify-center border border-muted-foreground/40",
                      question.multiSelect ? "rounded" : "rounded-full",
                      selected && "bg-sky-500 text-white",
                    )}
                  >
                    {selected && <Check className="h-3 w-3" />}
                  </span>
                  <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs">
                    <span
                      className={cn(
                        "truncate font-medium",
                        option.description ? "max-w-[55%]" : "min-w-0 flex-1",
                      )}
                    >
                      {localizedText(option.label, t)}
                    </span>
                    {option.description && (
                      <span className="min-w-0 flex-1 truncate text-[10px] font-normal text-muted-foreground">
                        {localizedText(option.description, t)}
                      </span>
                    )}
                    <span className="ml-auto shrink-0">
                      {option.recommended && (
                        <Star className="h-3 w-3 shrink-0 text-amber-500" aria-hidden="true" />
                      )}
                    </span>
                  </span>
                </Button>
              )
            })}
            {question.allowCustom && (
              <>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={submitting || timedOut || measuring}
                  aria-pressed={state?.customSelected ?? false}
                  onClick={() => toggleCustom(question)}
                  className={cn(
                    "h-8 w-full justify-start gap-1.5 rounded-lg px-2 text-xs hover:bg-muted/70",
                    state?.customSelected && "bg-muted",
                  )}
                >
                  <span
                    className={cn(
                      "flex h-3.5 w-3.5 shrink-0 items-center justify-center border border-muted-foreground/40",
                      question.multiSelect ? "rounded" : "rounded-full",
                      state?.customSelected && "bg-sky-500 text-white",
                    )}
                  >
                    {state?.customSelected && <Check className="h-3 w-3" />}
                  </span>
                  {t("common.other", { defaultValue: "Other" })}
                </Button>
                {state?.customSelected && (
                  <Input
                    autoFocus={!measuring}
                    surface="embedded"
                    disabled={submitting || timedOut || measuring}
                    value={state.customInput}
                    onChange={(event) => updateCustomInput(question.questionId, event.target.value)}
                    onKeyDown={onTextKeyDown}
                    placeholder={t("planMode.question.customPlaceholder", {
                      defaultValue: "Enter custom answer…",
                    })}
                    className="h-8 rounded-lg text-sm"
                  />
                )}
              </>
            )}
          </div>
        )}
      </div>

      {error && <p className="mt-1.5 text-[11px] text-destructive">{error}</p>}

      <footer className="mt-2 flex items-center gap-1.5">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled={questionIndex === 0 || submitting || measuring}
          onClick={() => {
            setError(null)
            setQuestionIndex((current) => Math.max(0, current - 1))
          }}
          aria-label={t("common.previous", { defaultValue: "Previous" })}
          className="h-7 w-7 rounded-full"
        >
          <ChevronLeft className="h-3.5 w-3.5" />
        </Button>
        <div className="flex min-w-0 flex-1 justify-center gap-1" aria-hidden="true">
          {group.questions.map((candidate, index) => (
            <span
              key={candidate.questionId}
              className={cn(
                "h-1 rounded-full transition-[width,background-color]",
                index === questionIndex ? "w-3 bg-sky-500" : "w-1 bg-muted-foreground/25",
              )}
            />
          ))}
        </div>
        <Button
          type="button"
          size="sm"
          disabled={!canContinue || submitting || measuring}
          onClick={next}
          className="h-7 gap-1 rounded-full px-2.5 text-xs"
        >
          {isLast ? (
            <>
              <Send className="h-3.5 w-3.5" />
              {t("planMode.question.submit", { defaultValue: "Submit" })}
            </>
          ) : (
            <>
              {t("common.next", { defaultValue: "Next" })}
              <ChevronRight className="h-3.5 w-3.5" />
            </>
          )}
        </Button>
      </footer>
    </section>
  )
}
