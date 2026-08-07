import { useEffect, useState, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTransport } from "@/lib/transport-provider"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import { basename } from "@/lib/path"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import {
  Check,
  ChevronRight,
  ClipboardList,
  FolderOpen,
  MessageCircleQuestion,
  PanelRight,
  Star,
  Timer,
} from "lucide-react"

type AskUserLocalizedText =
  | string
  | {
      key: string
      params?: Record<string, unknown>
      fallback?: string
    }

interface AskUserResultAnswer {
  questionId?: string
  question: string
  selected: string[]
  selectedValues?: string[]
  customInput?: string | null
}

interface AskUserOriginalOption {
  value: string
  label: AskUserLocalizedText
  description?: AskUserLocalizedText
  recommended?: boolean
}

interface AskUserOriginalQuestion {
  questionId?: string
  text?: AskUserLocalizedText
  question?: AskUserLocalizedText
  header?: AskUserLocalizedText
  options?: AskUserOriginalOption[]
  multi_select?: boolean
  multiSelect?: boolean
  default_values?: string[]
  defaultValues?: string[]
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function stringArray(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined
  return value.filter((item): item is string => typeof item === "string")
}

function parseLocalizedText(value: unknown): AskUserLocalizedText | undefined {
  if (typeof value === "string") return value
  if (!isRecord(value) || typeof value.key !== "string") return undefined
  return {
    key: value.key,
    params: isRecord(value.params) ? value.params : undefined,
    fallback: typeof value.fallback === "string" ? value.fallback : undefined,
  }
}

function parseOriginalOption(value: unknown): AskUserOriginalOption | null {
  if (!isRecord(value) || typeof value.value !== "string") return null
  const label = parseLocalizedText(value.label)
  if (!label) return null
  return {
    value: value.value,
    label,
    description: parseLocalizedText(value.description),
    recommended: value.recommended === true,
  }
}

function parseOriginalQuestion(value: unknown): AskUserOriginalQuestion {
  if (!isRecord(value)) return {}
  const rawOptions = Array.isArray(value.options) ? value.options : []
  return {
    questionId:
      typeof value.questionId === "string"
        ? value.questionId
        : typeof value.question_id === "string"
          ? value.question_id
          : undefined,
    text: parseLocalizedText(value.text),
    question: parseLocalizedText(value.question),
    header: parseLocalizedText(value.header),
    options: rawOptions
      .map(parseOriginalOption)
      .filter((option): option is AskUserOriginalOption => option !== null),
    multi_select: typeof value.multi_select === "boolean" ? value.multi_select : undefined,
    multiSelect: typeof value.multiSelect === "boolean" ? value.multiSelect : undefined,
    default_values: stringArray(value.default_values),
    defaultValues: stringArray(value.defaultValues),
  }
}

function parseResultAnswer(value: unknown): AskUserResultAnswer | null {
  if (!isRecord(value) || typeof value.question !== "string") return null
  const selected = stringArray(value.selected)
  if (!selected) return null
  return {
    questionId: typeof value.questionId === "string" ? value.questionId : undefined,
    question: value.question,
    selected,
    selectedValues: stringArray(value.selectedValues),
    customInput:
      typeof value.customInput === "string" || value.customInput === null
        ? value.customInput
        : undefined,
  }
}

function fallbackText(text: AskUserLocalizedText | undefined | null): string {
  if (!text) return ""
  if (typeof text === "string") return text
  return text.fallback || text.key
}

function localizedText(
  text: AskUserLocalizedText | undefined | null,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!text) return ""
  if (typeof text === "string") return text
  return t(text.key, {
    ...(text.params ?? {}),
    defaultValue: text.fallback || text.key,
  })
}

/** Collapsible Q&A summary for ask_user_question tool results.
 *
 * `pending=true` is rendered while the tool is still in flight (no result yet),
 * so the user sees that the model has dispatched a question instead of staring
 * at an empty bubble. The actual interaction happens in the AskUserDialog,
 * which is wired to a separate event channel — this card is purely a passive
 * indicator on the message timeline. */
export function AskUserQuestionResult({
  result,
  toolArguments,
  pending = false,
}: {
  result?: string
  toolArguments?: string
  pending?: boolean
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(true)

  const outcome = useMemo(() => {
    if (!result) return { items: [], timedOut: false }
    try {
      const data: unknown = JSON.parse(result)
      if (!isRecord(data)) return { items: [], timedOut: false }
      return {
        items: Array.isArray(data.answers)
          ? data.answers
              .map(parseResultAnswer)
              .filter((answer): answer is AskUserResultAnswer => answer !== null)
          : [],
        timedOut: data.timedOut === true,
      }
    } catch {
      return { items: [], timedOut: false }
    }
  }, [result])
  const items = outcome.items

  const original = useMemo(() => {
    if (!toolArguments) return null
    try {
      const data: unknown = JSON.parse(toolArguments)
      if (!isRecord(data)) return null
      return {
        context: parseLocalizedText(data.context),
        questions: Array.isArray(data.questions) ? data.questions.map(parseOriginalQuestion) : [],
      }
    } catch {
      return null
    }
  }, [toolArguments])

  if (pending) {
    // Use the same shimmer style as ToolCallBlock running state for visual
    // consistency — the rest of the app uses animate-pulse / text-shimmer
    // for in-flight indicators, never spinner. The icon pulses subtly while
    // the label text gets the sweeping shimmer treatment.
    return (
      <div className="my-2 flex items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-2.5 text-sm text-amber-700 dark:text-amber-400">
        <MessageCircleQuestion className="h-4 w-4 shrink-0 animate-pulse" />
        <span className="font-medium animate-text-shimmer">{t("planMode.question.pending")}</span>
      </div>
    )
  }

  if (items.length === 0) return null

  const originalQuestionFor = (item: AskUserResultAnswer, index: number) =>
    (item.questionId
      ? original?.questions.find((question) => question.questionId === item.questionId)
      : undefined) ?? original?.questions[index]

  const displaySelected = (item: AskUserResultAnswer, question?: AskUserOriginalQuestion) =>
    (item.selectedValues ?? item.selected).map((selected, index) => {
      const matchingOption = question?.options?.find(
        (option) =>
          option.value === selected ||
          (item.selectedValues === undefined && fallbackText(option.label) === selected),
      )
      return matchingOption
        ? localizedText(matchingOption.label, t)
        : (item.selected[index] ?? selected)
    })

  const answerPreview = items
    .flatMap((item, index) => [
      ...displaySelected(item, originalQuestionFor(item, index)),
      ...(item.customInput ? [item.customInput] : []),
    ])
    .join(" · ")

  return (
    <div className="my-2 rounded-lg border border-green-500/20 bg-green-500/5">
      <button
        type="button"
        aria-expanded={expanded}
        className="flex items-center gap-2 w-full px-4 py-2.5 text-sm text-green-600 hover:bg-green-500/5 transition-colors cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Check className="h-4 w-4" />
        <span className="font-medium">{t("planMode.question.answered")}</span>
        {outcome.timedOut && (
          <span className="rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-normal text-amber-700 dark:text-amber-400">
            {t("planMode.question.timedOut", { defaultValue: "timed out" })}
          </span>
        )}
        {!expanded && answerPreview && (
          <span className="ml-auto min-w-0 max-w-[65%] truncate text-xs font-normal text-muted-foreground">
            {answerPreview}
          </span>
        )}
      </button>
      <AnimatedCollapse open={expanded}>
        <div className="border-t border-green-500/10 px-4 pb-4 pt-3">
          {original?.context && (
            <div className="mb-3 flex items-start gap-2 text-xs text-muted-foreground">
              <MessageCircleQuestion className="mt-0.5 h-3.5 w-3.5 shrink-0 text-green-600/70" />
              <span className="whitespace-pre-wrap">{localizedText(original.context, t)}</span>
            </div>
          )}

          <div className="space-y-4">
            {items.map((item, i) => {
              const question = originalQuestionFor(item, i)
              const options = question?.options ?? []
              const defaultValues = question?.default_values ?? question?.defaultValues ?? []
              const selected = displaySelected(item, question)
              const questionText =
                localizedText(question?.text ?? question?.question, t) || item.question

              return (
                <section key={`${item.question}-${i}`} className="text-xs">
                  <div className="flex items-start gap-2">
                    <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-green-500/10 px-1 text-[10px] font-semibold text-green-700 dark:text-green-400">
                      {i + 1}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="font-medium leading-5 text-foreground">
                          {questionText}
                        </span>
                        {question?.header && (
                          <span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                            {localizedText(question.header, t)}
                          </span>
                        )}
                        {(question?.multi_select || question?.multiSelect) && (
                          <span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                            {t("planMode.question.multiSelect", { defaultValue: "multi" })}
                          </span>
                        )}
                      </div>

                      {options.length > 0 && (
                        <div className="mt-2.5">
                          <div className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/70">
                            {t("planMode.question.options")}
                          </div>
                          <div className="space-y-1.5">
                            {options.map((option) => {
                              const optionLabel = localizedText(option.label, t)
                              const isSelected = item.selectedValues
                                ? item.selectedValues.includes(option.value)
                                : item.selected.some(
                                    (value) =>
                                      value === fallbackText(option.label) ||
                                      value === option.value,
                                  )
                              return (
                                <div
                                  key={option.value}
                                  className={cn(
                                    "flex items-start gap-2 rounded-md border border-border/50 px-2.5 py-2 text-foreground",
                                    isSelected ? "bg-green-500/10" : "bg-background/30",
                                  )}
                                >
                                  <span
                                    className={cn(
                                      "mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border border-muted-foreground/30 text-white",
                                      isSelected ? "bg-green-600" : "bg-transparent",
                                    )}
                                  >
                                    {isSelected && <Check className="h-2.5 w-2.5" />}
                                  </span>
                                  <div className="min-w-0 flex-1">
                                    <div className="flex flex-wrap items-center gap-1.5">
                                      <span className="font-medium">{optionLabel}</span>
                                      {option.recommended && (
                                        <span className="inline-flex items-center gap-0.5 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-700 dark:text-amber-400">
                                          <Star className="h-2.5 w-2.5" />
                                          {t("planMode.question.recommended")}
                                        </span>
                                      )}
                                      {defaultValues.includes(option.value) && (
                                        <span className="inline-flex items-center gap-0.5 rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                          <Timer className="h-2.5 w-2.5" />
                                          {t("planMode.question.default", {
                                            defaultValue: "default",
                                          })}
                                        </span>
                                      )}
                                    </div>
                                    {option.description && (
                                      <div className="mt-0.5 leading-relaxed text-muted-foreground">
                                        {localizedText(option.description, t)}
                                      </div>
                                    )}
                                  </div>
                                </div>
                              )
                            })}
                          </div>
                        </div>
                      )}

                      <div className="mt-2.5 rounded-md border border-green-500/15 bg-green-500/5 px-2.5 py-2">
                        <div className="text-[10px] font-medium uppercase tracking-wide text-green-700/80 dark:text-green-400/80">
                          {outcome.timedOut
                            ? defaultValues.length > 0
                              ? t("tools.ask_user.timed_out")
                              : t("planMode.question.timedOut", { defaultValue: "timed out" })
                            : t("planMode.question.response")}
                        </div>
                        {selected.length > 0 ? (
                          <div className="mt-1.5 flex flex-wrap gap-1.5">
                            {selected.map((value, j) => (
                              <span
                                key={`${value}-${j}`}
                                className="inline-flex items-center gap-1 rounded-full bg-green-500/15 px-2 py-0.5 font-medium text-green-700 dark:text-green-300"
                              >
                                <Check className="h-2.5 w-2.5" />
                                {value}
                              </span>
                            ))}
                          </div>
                        ) : !item.customInput ? (
                          <div className="mt-1 text-muted-foreground">
                            {t("tools.ask_user.no_answers")}
                          </div>
                        ) : null}
                        {item.customInput && (
                          <div className={cn(selected.length > 0 && "mt-2")}>
                            <div className="text-[10px] text-muted-foreground">
                              {t("planMode.question.customAnswer")}
                            </div>
                            <div className="mt-1 whitespace-pre-wrap break-words rounded bg-background/60 px-2 py-1.5 leading-relaxed text-foreground">
                              {item.customInput}
                            </div>
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                </section>
              )
            })}
          </div>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

/** Compact inline card for submit_plan tool calls.
 *
 * `pending=true` renders a shimmer chip while the tool is in flight. The
 * normal card (with reveal + open-panel buttons) only appears once the model
 * has actually written and saved the plan file. This is the user's primary
 * feedback that the plan write is happening — without it, the bubble used to
 * be completely empty between dispatch and result. */
export function SubmitPlanResult({
  title,
  sessionId,
  onOpenPanel,
  pending = false,
}: {
  title: string
  sessionId?: string | null
  onOpenPanel?: () => void
  pending?: boolean
}) {
  const { t } = useTranslation()
  const transport = useTransport()
  const [pendingRevealPath, setPendingRevealPath] = useState<string | null>(null)
  const planTarget = useMemo<PreviewTarget | null>(
    () =>
      sessionId
        ? {
            kind: "sessionPath",
            sessionId,
            path: pendingRevealPath ?? "",
            name: pendingRevealPath ? basename(pendingRevealPath) : "plan.md",
          }
        : null,
    [pendingRevealPath, sessionId],
  )
  const planFileOverrides = useMemo(() => ({ sessionId }), [sessionId])
  const planFileActions = useFileResource(planTarget, planFileOverrides)
  const runPlanFileAction = planFileActions.run
  const canReveal = planTarget != null && planFileActions.capabilities.reveal.state === "enabled"

  useEffect(() => {
    if (!pendingRevealPath) return
    const requestedPath = pendingRevealPath
    void runPlanFileAction("reveal").finally(() =>
      setPendingRevealPath((current) => (current === requestedPath ? null : current)),
    )
  }, [pendingRevealPath, runPlanFileAction])

  if (pending) {
    // Shimmer-style indicator (same as ToolCallBlock / ThinkingBlock running
    // state) — keeps the in-flight visual language consistent across the
    // whole bubble.
    return (
      <div className="my-2 flex items-center gap-2 rounded-lg border border-purple-500/20 bg-purple-500/5 px-4 py-2.5 text-sm text-purple-700 dark:text-purple-400">
        <ClipboardList className="h-4 w-4 shrink-0 animate-pulse" />
        <span className="font-medium truncate flex-1 animate-text-shimmer">
          {title || t("planMode.submittingPlan")}
        </span>
      </div>
    )
  }

  const handleRevealFile = async () => {
    if (!sessionId) return
    const path = await transport
      .call<string | null>("get_plan_file_path", { sessionId })
      .catch(() => null)
    if (path) setPendingRevealPath(path)
  }

  return (
    <div
      className="my-2 rounded-lg border border-purple-500/20 bg-purple-500/5 px-4 py-3 flex items-center gap-3 cursor-pointer hover:bg-purple-500/10 transition-colors"
      onClick={onOpenPanel}
    >
      <ClipboardList className="h-4 w-4 text-purple-600 shrink-0" />
      <span className="text-sm font-medium truncate flex-1">
        {title || t("planMode.panelTitle")}
      </span>
      <div className="flex items-center gap-1.5 shrink-0">
        <IconTip label={t("planMode.openPanel")}>
          <button
            onClick={(e) => {
              e.stopPropagation()
              onOpenPanel?.()
            }}
            className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
          >
            <PanelRight className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        {canReveal ? (
          <IconTip label={t("chat.revealInFolder")}>
            <button
              onClick={(e) => {
                e.stopPropagation()
                void handleRevealFile()
              }}
              className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
            >
              <FolderOpen className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        ) : null}
      </div>
    </div>
  )
}
