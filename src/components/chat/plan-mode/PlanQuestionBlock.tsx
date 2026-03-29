import { useState, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { HelpCircle, Check, Send, MessageSquare, Star, Target, Layers, AlertTriangle } from "lucide-react"

export interface PlanQuestionOption {
  value: string
  label: string
  description?: string
  recommended?: boolean
}

export interface PlanQuestion {
  questionId: string
  text: string
  options: PlanQuestionOption[]
  allowCustom: boolean
  multiSelect: boolean
  template?: string
}

export interface PlanQuestionGroup {
  requestId: string
  sessionId: string
  questions: PlanQuestion[]
  context?: string
}

export interface PlanQuestionAnswer {
  questionId: string
  selected: string[]
  customInput?: string
}

interface PlanQuestionBlockProps {
  group: PlanQuestionGroup
  onSubmitted?: () => void
}

interface QuestionState {
  selected: Set<string>
  customInput: string
}

export default function PlanQuestionBlock({ group, onSubmitted }: PlanQuestionBlockProps) {
  const { t } = useTranslation()
  const [submitted, setSubmitted] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [answers, setAnswers] = useState<Record<string, QuestionState>>(() => {
    const init: Record<string, QuestionState> = {}
    for (const q of group.questions) {
      init[q.questionId] = { selected: new Set(), customInput: "" }
    }
    return init
  })

  const toggleOption = useCallback((questionId: string, value: string, multiSelect: boolean) => {
    setAnswers(prev => {
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
      return { ...prev, [questionId]: { ...q, selected: newSelected } }
    })
  }, [])

  const setCustomInput = useCallback((questionId: string, value: string) => {
    setAnswers(prev => {
      const q = prev[questionId]
      if (!q) return prev
      return { ...prev, [questionId]: { ...q, customInput: value } }
    })
  }, [])

  const handleSubmit = useCallback(async () => {
    setSubmitting(true)
    setError(null)
    try {
      const answerList: PlanQuestionAnswer[] = group.questions.map(q => {
        const state = answers[q.questionId]
        return {
          questionId: q.questionId,
          selected: state ? Array.from(state.selected) : [],
          customInput: state?.customInput || undefined,
        }
      })
      await invoke("respond_plan_question", {
        requestId: group.requestId,
        answers: answerList,
      })
      setSubmitted(true)
      onSubmitted?.()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.error("plan", "PlanQuestionBlock::submit", "Failed to submit plan question response", msg)
      setError(msg)
    } finally {
      setSubmitting(false)
    }
  }, [group, answers, onSubmitted])

  // After submission, the Q&A summary is rendered inline by PlanQuestionResult
  // in the tool call's position within the message flow
  if (submitted) return null

  return (
    <div className="my-2 rounded-lg border border-blue-500/20 bg-blue-500/5 p-4 space-y-4">
      {/* Header */}
      <div className="flex items-center gap-2 text-sm font-medium text-blue-600">
        <MessageSquare className="h-4 w-4" />
        <span>{t("planMode.question.title")}</span>
      </div>

      {/* Context */}
      {group.context && (
        <p className="text-sm text-muted-foreground">{group.context}</p>
      )}

      {/* Questions */}
      {group.questions.map((q, qi) => (
        <div key={q.questionId} className="space-y-2">
          <div className="flex items-start gap-2">
            {q.template === "scope" ? <Target className="h-3.5 w-3.5 mt-0.5 text-purple-500 shrink-0" />
              : q.template === "tech_choice" ? <Layers className="h-3.5 w-3.5 mt-0.5 text-green-500 shrink-0" />
              : q.template === "priority" ? <AlertTriangle className="h-3.5 w-3.5 mt-0.5 text-amber-500 shrink-0" />
              : <HelpCircle className="h-3.5 w-3.5 mt-0.5 text-blue-500 shrink-0" />}
            <span className="text-sm font-medium">
              {group.questions.length > 1 && `${qi + 1}. `}{q.text}
            </span>
          </div>

          {/* Options */}
          <div className="pl-5 space-y-1.5">
            {q.options.map(opt => {
              const isSelected = answers[q.questionId]?.selected.has(opt.value)
              return (
                <button
                  key={opt.value}
                  onClick={() => toggleOption(q.questionId, opt.value, q.multiSelect)}
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
                    <div className={cn(
                      "h-4 w-4 rounded-full border-2 flex items-center justify-center shrink-0",
                      q.multiSelect ? "rounded-sm" : "",
                      isSelected ? "border-blue-500 bg-blue-500" : "border-muted-foreground/30"
                    )}>
                      {isSelected && <Check className="h-2.5 w-2.5 text-white" />}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="font-medium">{opt.label}</span>
                        {opt.recommended && (
                          <span className="inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-600">
                            <Star className="h-2.5 w-2.5" />
                            {t("planMode.question.recommended")}
                          </span>
                        )}
                      </div>
                      {opt.description && (
                        <div className="text-xs text-muted-foreground mt-0.5">{opt.description}</div>
                      )}
                    </div>
                  </div>
                </button>
              )
            })}

            {/* Custom input */}
            {q.allowCustom && (
              <div className="flex gap-2 mt-1">
                <Input
                  placeholder={t("planMode.question.customPlaceholder")}
                  value={answers[q.questionId]?.customInput || ""}
                  onChange={e => setCustomInput(q.questionId, e.target.value)}
                  className="text-sm h-9"
                />
              </div>
            )}
          </div>
        </div>
      ))}

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
          disabled={submitting}
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
