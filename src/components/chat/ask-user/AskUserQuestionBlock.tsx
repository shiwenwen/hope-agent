import { useState, useCallback, useEffect, useMemo } from "react"
import { Streamdown } from "streamdown"
import { code } from "@streamdown/code"
import { cjk } from "@streamdown/cjk"
import "streamdown/styles.css"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  HelpCircle,
  Check,
  Send,
  MessageSquare,
  Star,
  Target,
  Layers,
  AlertTriangle,
  Timer,
} from "lucide-react"

// ── Types (mirror of ha-core `AskUserQuestion*` types) ──

export interface AskUserQuestionOption {
  value: string
  label: string
  description?: string
  recommended?: boolean
  /** Rich preview body (markdown by default, or image URL / mermaid source). */
  preview?: string
  previewKind?: "markdown" | "image" | "mermaid"
}

export interface AskUserQuestion {
  questionId: string
  text: string
  options: AskUserQuestionOption[]
  /**
   * Whether to offer a free-form custom input. The backend currently forces
   * this to `true` at parse time (模型给的选项常常覆盖不到用户真实意图,
   * 强制留自由文本入口避免被迫二选一)，前端通过“其他”选项显式展开输入框。
   */
  allowCustom: boolean
  multiSelect: boolean
  template?: string
  /** Very short chip label (<=12 chars). */
  header?: string
  /** Per-question timeout in seconds. 0 / missing = inherit group default. */
  timeoutSecs?: number
  /** Values auto-selected if the question times out. */
  defaultValues?: string[]
}

export interface AskUserQuestionGroup {
  requestId: string
  sessionId: string
  questions: AskUserQuestion[]
  context?: string
  source?: string
  /** Unix timestamp (seconds) after which pending answers auto-fall back. */
  timeoutAt?: number
}

export interface AskUserQuestionAnswer {
  questionId: string
  selected: string[]
  customInput?: string
}

interface AskUserQuestionBlockProps {
  group: AskUserQuestionGroup
  onSubmitted?: () => void
}

interface QuestionState {
  selected: Set<string>
  customSelected: boolean
  customInput: string
}

// ── Lightweight preview renderer (no streaming, no rAF) ──────────

const staticPlugins = { code, cjk }
const CUSTOM_OPTION_FOCUS = "__custom__"

function OptionPreview({ option }: { option: AskUserQuestionOption }) {
  const kind = option.previewKind ?? "markdown"
  const preview = option.preview ?? ""
  if (!preview) return null

  if (kind === "image") {
    return (
      <div className="mt-2 rounded-md border border-border overflow-hidden">
        <img
          src={preview}
          alt={option.label}
          className="max-h-64 w-full object-contain bg-muted"
          loading="lazy"
        />
      </div>
    )
  }

  const body = kind === "mermaid" ? "```mermaid\n" + preview + "\n```" : preview
  return (
    <div className="mt-2 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs max-h-[28rem] overflow-auto">
      <Streamdown plugins={staticPlugins}>{body}</Streamdown>
    </div>
  )
}

// ── Countdown timer ──────────────────────────────────────────────

function useCountdown(timeoutAt: number | undefined | null) {
  const [remaining, setRemaining] = useState<number | null>(null)

  useEffect(() => {
    if (!timeoutAt) {
      const id = window.setTimeout(() => setRemaining(null), 0)
      return () => window.clearTimeout(id)
    }
    let timer: number | undefined
    const tick = () => {
      const secs = Math.max(0, timeoutAt - Math.floor(Date.now() / 1000))
      setRemaining(secs)
      if (secs <= 0 && timer !== undefined) {
        window.clearInterval(timer)
        timer = undefined
      }
    }
    const first = window.setTimeout(tick, 0)
    timer = window.setInterval(tick, 1000)
    return () => {
      if (timer !== undefined) window.clearInterval(timer)
      window.clearTimeout(first)
    }
  }, [timeoutAt])

  return remaining
}

function formatRemaining(secs: number): string {
  if (secs <= 0) return "0s"
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  const s = secs % 60
  if (m < 60) return `${m}m ${s}s`
  const h = Math.floor(m / 60)
  return `${h}h ${m % 60}m`
}

// ── Main component ───────────────────────────────────────────────

export default function AskUserQuestionBlock({ group, onSubmitted }: AskUserQuestionBlockProps) {
  const { t } = useTranslation()
  const [submitted, setSubmitted] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [answers, setAnswers] = useState<Record<string, QuestionState>>(() => {
    const init: Record<string, QuestionState> = {}
    for (const q of group.questions) {
      init[q.questionId] = {
        selected: new Set<string>(),
        customSelected: false,
        customInput: "",
      }
    }
    return init
  })
  const [focusedOption, setFocusedOption] = useState<Record<string, string>>({})
  const otherLabel = t("common.other", { defaultValue: "Other" })

  const remaining = useCountdown(group.timeoutAt)
  const hasAnyPreview = useMemo(
    () => group.questions.some((q) => q.options.some((o) => !!o.preview)),
    [group.questions]
  )

  const toggleOption = useCallback(
    (questionId: string, value: string, multiSelect: boolean) => {
      setAnswers((prev) => {
        const q = prev[questionId]
        if (!q) return prev
        const newSelected = new Set(q.selected)
        if (multiSelect) {
          if (newSelected.has(value)) newSelected.delete(value)
          else newSelected.add(value)
        } else {
          newSelected.clear()
          newSelected.add(value)
        }
        return {
          ...prev,
          [questionId]: {
            ...q,
            selected: newSelected,
            customSelected: multiSelect ? q.customSelected : false,
          },
        }
      })
      setFocusedOption((prev) => ({ ...prev, [questionId]: value }))
    },
    []
  )

  const toggleCustomOption = useCallback(
    (questionId: string, multiSelect: boolean) => {
      setAnswers((prev) => {
        const q = prev[questionId]
        if (!q) return prev
        return {
          ...prev,
          [questionId]: {
            ...q,
            selected: multiSelect ? new Set(q.selected) : new Set<string>(),
            customSelected: multiSelect ? !q.customSelected : true,
          },
        }
      })
      setFocusedOption((prev) => ({ ...prev, [questionId]: CUSTOM_OPTION_FOCUS }))
    },
    []
  )

  const setCustomInput = useCallback((questionId: string, value: string) => {
    setAnswers((prev) => {
      const q = prev[questionId]
      if (!q) return prev
      return { ...prev, [questionId]: { ...q, customInput: value } }
    })
  }, [])

  const handleSubmit = useCallback(async () => {
    setError(null)
    const missingCustom = group.questions.find((q) => {
      const state = answers[q.questionId]
      return state?.customSelected && !state.customInput.trim()
    })
    if (missingCustom) {
      setFocusedOption((prev) => ({
        ...prev,
        [missingCustom.questionId]: CUSTOM_OPTION_FOCUS,
      }))
      setError(t("planMode.question.customRequired"))
      return
    }

    setSubmitting(true)
    try {
      const answerList: AskUserQuestionAnswer[] = group.questions.map((q) => {
        const state = answers[q.questionId]
        const customInput = state?.customSelected ? state.customInput.trim() : ""
        return {
          questionId: q.questionId,
          selected: state ? Array.from(state.selected) : [],
          customInput: customInput || undefined,
        }
      })
      await getTransport().call("respond_ask_user_question", {
        requestId: group.requestId,
        answers: answerList,
      })
      setSubmitted(true)
      onSubmitted?.()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.error(
        "ask_user",
        "AskUserQuestionBlock::submit",
        "Failed to submit ask_user response",
        msg
      )
      setError(msg)
    } finally {
      setSubmitting(false)
    }
  }, [group, answers, onSubmitted, t])

  if (submitted) return null

  const timedOut = remaining !== null && remaining <= 0
  const lowTime = remaining !== null && remaining > 0 && remaining <= 10

  return (
    <div className="my-2 rounded-lg border border-blue-500/20 bg-blue-500/5 p-4 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-medium text-blue-600">
          <MessageSquare className="h-4 w-4" />
          <span>{t("planMode.question.title")}</span>
        </div>
        {remaining !== null && (
          <IconTip label={t("planMode.question.timeoutHint", { defaultValue: "Time remaining" })}>
            <div
              className={cn(
                "flex items-center gap-1 text-xs rounded-full px-2 py-0.5",
                timedOut
                  ? "bg-destructive/10 text-destructive"
                  : lowTime
                    ? "bg-amber-500/15 text-amber-600 animate-pulse"
                    : "bg-muted text-muted-foreground"
              )}
            >
              <Timer className="h-3 w-3" />
              <span>{timedOut ? t("planMode.question.timedOut", { defaultValue: "timed out" }) : formatRemaining(remaining)}</span>
            </div>
          </IconTip>
        )}
      </div>

      {/* Context */}
      {group.context && <p className="text-sm text-muted-foreground">{group.context}</p>}

      {/* Questions */}
      {group.questions.map((q, qi) => {
        const state = answers[q.questionId]
        const focused = focusedOption[q.questionId]
        const focusedOpt = q.options.find((o) => o.value === focused)
        const showSidePreview = hasAnyPreview && !!focusedOpt?.preview
        const customSelected = state?.customSelected ?? false
        return (
          <div
            key={q.questionId}
            className={cn(
              "space-y-2",
              showSidePreview &&
                "md:grid md:grid-cols-[minmax(260px,2fr)_3fr] md:gap-4 md:space-y-0"
            )}
          >
            {/* Left column: title + options */}
            <div className="space-y-2">
              <div className="flex items-start gap-2 flex-wrap">
                {q.template === "scope" ? (
                  <Target className="h-3.5 w-3.5 mt-0.5 text-purple-500 shrink-0" />
                ) : q.template === "tech_choice" ? (
                  <Layers className="h-3.5 w-3.5 mt-0.5 text-green-500 shrink-0" />
                ) : q.template === "priority" ? (
                  <AlertTriangle className="h-3.5 w-3.5 mt-0.5 text-amber-500 shrink-0" />
                ) : (
                  <HelpCircle className="h-3.5 w-3.5 mt-0.5 text-blue-500 shrink-0" />
                )}
                <span className="text-sm font-medium">
                  {group.questions.length > 1 && `${qi + 1}. `}
                  {q.text}
                </span>
                {q.header && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-blue-500/10 text-blue-600 font-normal uppercase tracking-wide">
                    {q.header}
                  </span>
                )}
                {q.multiSelect && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground font-normal">
                    {t("planMode.question.multiSelect", { defaultValue: "multi" })}
                  </span>
                )}
              </div>

              <div className="pl-5 space-y-1.5">
                {q.options.map((opt) => {
                  const isSelected = state?.selected.has(opt.value) ?? false
                  const isDefault =
                    q.defaultValues?.includes(opt.value) ?? false
                  return (
                    <button
                      key={opt.value}
                      onClick={() => toggleOption(q.questionId, opt.value, q.multiSelect)}
                      onMouseEnter={() =>
                        setFocusedOption((prev) => ({ ...prev, [q.questionId]: opt.value }))
                      }
                      className={cn(
                        "w-full text-left px-3 py-2 rounded-md border text-sm transition-colors cursor-pointer",
                        isSelected
                          ? "border-blue-500 bg-blue-500/10 text-blue-700 dark:text-blue-300"
                          : opt.recommended
                            ? "border-amber-500/40 bg-amber-500/5 hover:border-amber-500/60"
                            : "border-border hover:border-blue-500/50 hover:bg-blue-500/5"
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "h-4 w-4 border-2 flex items-center justify-center shrink-0",
                            q.multiSelect ? "rounded-sm" : "rounded-full",
                            isSelected
                              ? "border-blue-500 bg-blue-500"
                              : "border-muted-foreground/30"
                          )}
                        >
                          {isSelected && <Check className="h-2.5 w-2.5 text-white" />}
                        </div>
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-1.5 flex-wrap">
                            <span className="font-medium">{opt.label}</span>
                            {opt.recommended && (
                              <span className="inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-600">
                                <Star className="h-2.5 w-2.5" />
                                {t("planMode.question.recommended")}
                              </span>
                            )}
                            {isDefault && (
                              <span className="inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground">
                                <Timer className="h-2.5 w-2.5" />
                                {t("planMode.question.default", { defaultValue: "default" })}
                              </span>
                            )}
                          </div>
                          {opt.description && (
                            <div className="text-xs text-muted-foreground mt-0.5">
                              {opt.description}
                            </div>
                          )}
                          {/* Inline preview (when no side pane is active for this question) */}
                          {opt.preview && !hasAnyPreview && <OptionPreview option={opt} />}
                        </div>
                      </div>
                    </button>
                  )
                })}

                {/* Custom input is gated behind an explicit "Other" choice so
                    regular selections and free-form answers don't blur together. */}
                {q.allowCustom && (
                  <>
                    <button
                      type="button"
                      onClick={() => toggleCustomOption(q.questionId, q.multiSelect)}
                      onMouseEnter={() =>
                        setFocusedOption((prev) => ({
                          ...prev,
                          [q.questionId]: CUSTOM_OPTION_FOCUS,
                        }))
                      }
                      className={cn(
                        "w-full text-left px-3 py-2 rounded-md border text-sm transition-colors cursor-pointer",
                        customSelected
                          ? "border-blue-500 bg-blue-500/10 text-blue-700 dark:text-blue-300"
                          : "border-border hover:border-blue-500/50 hover:bg-blue-500/5"
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "h-4 w-4 border-2 flex items-center justify-center shrink-0",
                            q.multiSelect ? "rounded-sm" : "rounded-full",
                            customSelected
                              ? "border-blue-500 bg-blue-500"
                              : "border-muted-foreground/30"
                          )}
                        >
                          {customSelected && <Check className="h-2.5 w-2.5 text-white" />}
                        </div>
                        <span className="font-medium">{otherLabel}</span>
                      </div>
                    </button>
                    {customSelected && (
                      <div className="flex gap-2 mt-1">
                        <Input
                          placeholder={t("planMode.question.customPlaceholder")}
                          value={state?.customInput || ""}
                          onChange={(e) => setCustomInput(q.questionId, e.target.value)}
                          className="text-sm h-9"
                        />
                      </div>
                    )}
                  </>
                )}
              </div>
            </div>

            {/* Right column: side preview pane aligned to the question's top */}
            {showSidePreview && (
              <div className="hidden md:block">
                <div className="flex items-center text-[10px] uppercase tracking-wide text-muted-foreground mb-1 leading-5 h-5">
                  {t("planMode.question.preview", { defaultValue: "Preview" })}: {focusedOpt!.label}
                </div>
                <OptionPreview option={focusedOpt!} />
              </div>
            )}
          </div>
        )
      })}

      {/* Error display */}
      {error && (
        <div className="text-xs text-destructive bg-destructive/10 rounded-md px-3 py-2">
          {error}
        </div>
      )}

      {/* Submit button */}
      <div className="flex justify-end pt-1">
        <Button
          size="sm"
          onClick={handleSubmit}
          disabled={submitting || timedOut}
          className={cn("gap-1.5", error && "bg-destructive/10 text-destructive hover:bg-destructive/20")}
        >
          {submitting ? (
            <span className="animate-spin h-3.5 w-3.5 border-2 border-current border-t-transparent rounded-full" />
          ) : (
            <Send className="h-3.5 w-3.5" />
          )}
          {error ? t("planMode.question.retry") : t("planMode.question.submit")}
        </Button>
      </div>
    </div>
  )
}
