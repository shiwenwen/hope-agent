import { useState, useCallback, useMemo } from "react"
import { code } from "@streamdown/code"
import { cjk } from "@streamdown/cjk"
import "streamdown/styles.css"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { formatRemaining, useCountdownRemainingSec } from "@/lib/countdown"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { MarkdownStreamdown } from "@/components/common/MarkdownRenderer"
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

export type AskUserLocalizedText =
  | string
  | {
      key: string
      params?: Record<string, unknown>
      fallback?: string
    }

/** Visual "design direction" card payload carried by a `direction-cards`
 *  option — rendered richly only in the design chat (`variant="design"`),
 *  ignored (plain row) elsewhere. Answer is still the option `value`. */
export interface AskUserDirectionCard {
  palette?: string[]
  displayFont?: string
  bodyFont?: string
  mood?: AskUserLocalizedText
  references?: string[]
}

export interface AskUserQuestionOption {
  value: string
  label: AskUserLocalizedText
  description?: AskUserLocalizedText
  recommended?: boolean
  /** Rich preview body (markdown by default, or image URL / mermaid source). */
  preview?: string
  previewKind?: "markdown" | "image" | "mermaid"
  card?: AskUserDirectionCard
}

export interface AskUserQuestion {
  questionId: string
  text: AskUserLocalizedText
  options: AskUserQuestionOption[]
  /**
   * Whether to offer a free-form custom input. The backend currently forces
   * this to `true` at parse time (模型给的选项常常覆盖不到用户真实意图,
   * 强制留自由文本入口避免被迫二选一)，前端通过“其他”选项显式展开输入框。
   */
  allowCustom: boolean
  multiSelect: boolean
  /** Primary input shape. Omitted = legacy single/multi (by `multiSelect`).
   *  `text`/`textarea` = free-text; `direction-cards` = visual style picker. */
  inputKind?: "single" | "multi" | "text" | "textarea" | "direction-cards"
  template?: string
  /** Very short chip label (<=12 chars). */
  header?: AskUserLocalizedText
  /** Per-question timeout in seconds. 0 / missing = inherit group default. */
  timeoutSecs?: number
  /** Values auto-selected if the question times out. */
  defaultValues?: string[]
}

export interface AskUserQuestionGroup {
  requestId: string
  sessionId: string
  questions: AskUserQuestion[]
  context?: AskUserLocalizedText
  source?: string
  /** Unix timestamp (seconds) after which pending answers auto-fall back. */
  timeoutAt?: number
  /** Effective duration used to derive timeoutAt; retained across restart. */
  timeoutSecs?: number
  /** Server UNIX time (seconds) when this payload was emitted/read. */
  serverNow?: number
  /** Client-clock deadline derived from timeoutAt + serverNow. */
  localTimeoutAtMs?: number | null
}

export interface AskUserQuestionAnswer {
  questionId: string
  selected: string[]
  customInput?: string
}

interface AskUserQuestionBlockProps {
  group: AskUserQuestionGroup
  onSubmitted?: () => void
  /** `"design"` unlocks the rich `direction-cards` visual style picker. Any
   *  other surface keeps the plain option list (safe degrade). */
  variant?: "default" | "design"
}

interface QuestionState {
  selected: Set<string>
  customSelected: boolean
  customInput: string
}

// ── Lightweight preview renderer (no streaming, no rAF) ──────────

const staticPlugins = { code, cjk }
const CUSTOM_OPTION_FOCUS = "__custom__"

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

// ── Direction cards (design-variant visual style picker) ─────────

/** Allowlist a model-supplied CSS color before it hits `style={{ background }}`.
 *  These cards render in the MAIN app (not the sandboxed artifact iframe), so a
 *  raw palette string is untrusted input — reject anything that isn't a #hex or
 *  a known functional color form to close the `url(...)` / CSS-injection vector.
 *  Unknown → `transparent` (the swatch still shows its border cell). */
const CSS_COLOR_RE =
  /^(#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color)\([0-9a-z.,%/ +-]*\))$/i
function sanitizeCssColor(raw: string): string {
  const s = raw.trim()
  return CSS_COLOR_RE.test(s) ? s : "transparent"
}

/** Sanitize a model-supplied font-family stack before inline style. Strips
 *  CSS-breaking chars so `fontFamily` can't smuggle a second declaration and
 *  caps length; empty / unsafe → undefined (browser default font). */
function sanitizeFontFamily(raw: string | undefined): string | undefined {
  if (!raw) return undefined
  const s = raw
    .replace(/url\s*\(/gi, "")
    .replace(/[;{}<>]/g, "")
    .trim()
    .slice(0, 200)
  return s || undefined
}

function DirectionCardView({
  option,
  selected,
  onSelect,
}: {
  option: AskUserQuestionOption
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const card = option.card ?? {}
  const palette = (card.palette ?? []).slice(0, 6).map(sanitizeCssColor)
  const displayFont = sanitizeFontFamily(card.displayFont)
  const bodyFont = sanitizeFontFamily(card.bodyFont)
  const mood = localizedText(card.mood, t)
  const refs = (card.references ?? []).slice(0, 4)
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-2 rounded-lg border p-3 text-left transition-colors cursor-pointer",
        selected
          ? "border-border bg-secondary"
          : "border-border hover:bg-secondary/40"
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium">{localizedText(option.label, t)}</span>
        {selected && (
          <span className="inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-600 shrink-0">
            <Check className="h-2.5 w-2.5" />
            {t("planMode.question.selected", { defaultValue: "Selected" })}
          </span>
        )}
      </div>
      {palette.length > 0 && (
        <div className="flex gap-1">
          {palette.map((c, i) => (
            <span
              key={i}
              className="h-5 flex-1 rounded-sm border border-border/40"
              style={{ background: c }}
            />
          ))}
        </div>
      )}
      {(displayFont || bodyFont) && (
        <div className="flex items-baseline gap-2 overflow-hidden">
          <span
            className="text-2xl leading-none shrink-0"
            style={displayFont ? { fontFamily: displayFont } : undefined}
          >
            Aa
          </span>
          <span
            className="text-xs text-muted-foreground truncate"
            style={bodyFont ? { fontFamily: bodyFont } : undefined}
          >
            {t("planMode.question.typeSample", { defaultValue: "The quick brown fox" })}
          </span>
        </div>
      )}
      {mood && <p className="text-xs text-muted-foreground line-clamp-2">{mood}</p>}
      {refs.length > 0 && (
        <p className="text-[11px] text-muted-foreground/80 truncate">
          {t("planMode.question.refs", { defaultValue: "Refs" })}: {refs.join(" · ")}
        </p>
      )}
    </button>
  )
}

function OptionPreview({
  option,
  fill = false,
}: {
  option: AskUserQuestionOption
  /** Fill the parent (side pane) instead of sizing to content — keeps the
      pane height constant so hovering between previews never reflows. */
  fill?: boolean
}) {
  const { t } = useTranslation()
  const kind = option.previewKind ?? "markdown"
  const preview = option.preview ?? ""
  if (!preview) return null

  if (kind === "image") {
    return (
      <div
        className={cn(
          "mt-2 rounded-md border border-border overflow-hidden",
          fill && "flex-1 min-h-0",
        )}
      >
        <img
          src={preview}
          alt={localizedText(option.label, t)}
          className={cn("w-full object-contain bg-muted", fill ? "h-full" : "max-h-64")}
          loading="lazy"
        />
      </div>
    )
  }

  const body = kind === "mermaid" ? "```mermaid\n" + preview + "\n```" : preview
  return (
    <div
      className={cn(
        "mt-2 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs overflow-auto",
        fill ? "flex-1 min-h-0" : "max-h-[28rem]",
      )}
    >
      <MarkdownStreamdown plugins={staticPlugins}>{body}</MarkdownStreamdown>
    </div>
  )
}

// ── Main component ───────────────────────────────────────────────

export default function AskUserQuestionBlock({
  group,
  onSubmitted,
  variant = "default",
}: AskUserQuestionBlockProps) {
  const { t } = useTranslation()

  // The `enter_plan_mode` tool uses this generic ask-user UI but its prompt
  // text (question / option labels / context prefix / "PLAN MODE" header) is
  // hardcoded English on the backend so IM channels and older clients still
  // get something sensible. In the desktop / web UI we override those four
  // pieces with i18n keys; the model-supplied `reason` (which the backend now
  // sends verbatim as `group.context`) is NOT translated — the model writes
  // it in the user's conversation language naturally.
  const isEnterPlanModeAsk =
    group.questions.length === 1 && group.questions[0]?.questionId === "enter_plan_mode"

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

  const remaining = useCountdownRemainingSec(
    group.localTimeoutAtMs ?? (group.timeoutAt ? group.timeoutAt * 1000 : null),
  )
  const hasAnyPreview = useMemo(
    () => group.questions.some((q) => q.options.some((o) => !!o.preview)),
    [group.questions],
  )

  const toggleOption = useCallback((questionId: string, value: string, multiSelect: boolean) => {
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
  }, [])

  const toggleCustomOption = useCallback((questionId: string, multiSelect: boolean) => {
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
  }, [])

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
        // Free-text questions have no options — the typed value IS the answer,
        // so it flows through `customInput` without the explicit "Other" toggle.
        const isFreeText = q.inputKind === "text" || q.inputKind === "textarea"
        const customInput =
          isFreeText || state?.customSelected ? (state?.customInput.trim() ?? "") : ""
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
        msg,
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
                    : "bg-muted text-muted-foreground",
              )}
            >
              <Timer className="h-3 w-3" />
              <span>
                {timedOut
                  ? t("planMode.question.timedOut", { defaultValue: "timed out" })
                  : formatRemaining(remaining)}
              </span>
            </div>
          </IconTip>
        )}
      </div>

      {/* Context */}
      {isEnterPlanModeAsk ? (
        <p className="text-sm text-muted-foreground">
          {fallbackText(group.context)
            ? t("planMode.enterDialog.contextPrefix", {
                reason: fallbackText(group.context),
              })
            : t("planMode.enterDialog.contextNoReason")}
        </p>
      ) : (
        group.context && (
          <p className="text-sm text-muted-foreground">{localizedText(group.context, t)}</p>
        )
      )}

      {/* Questions */}
      {group.questions.map((q, qi) => {
        const state = answers[q.questionId]
        const focused = focusedOption[q.questionId]
        const focusedOpt = q.options.find((o) => o.value === focused)
        // Side-preview pane content: the hovered option's own preview, or a
        // fallback to the first option that has one so the pane never
        // unmounts. Toggling the pane/grid on hover used to reflow the option
        // column width and made the option boxes jitter (issue #433); the
        // grid itself is reserved group-wide via `hasAnyPreview` so columns
        // stay aligned across questions too.
        const previewOpt = focusedOpt?.preview ? focusedOpt : q.options.find((o) => !!o.preview)
        // The fallback preview does not describe the focused option — dim it
        // so it reads as reference, not as the hovered option's detail.
        const previewIsFallback = !!previewOpt && previewOpt !== focusedOpt
        const customSelected = state?.customSelected ?? false
        const isFreeText = q.inputKind === "text" || q.inputKind === "textarea"
        // Rich style cards only in the design chat AND only when the model
        // actually attached card payloads — otherwise degrade to the plain list.
        const isDirectionCards =
          variant === "design" &&
          q.inputKind === "direction-cards" &&
          q.options.some((o) => o.card)
        const hasQuestionPreview = q.options.some((option) => !!option.preview)
        return (
          <div
            key={q.questionId}
            className={cn(
              "space-y-2",
              hasAnyPreview && "md:grid md:grid-cols-[minmax(260px,2fr)_3fr] md:gap-4 md:space-y-0",
              previewOpt && "md:min-h-64",
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
                  {isEnterPlanModeAsk
                    ? t("planMode.enterDialog.question")
                    : localizedText(q.text, t)}
                </span>
                {(isEnterPlanModeAsk || q.header) && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-blue-500/10 text-blue-600 font-normal uppercase tracking-wide">
                    {isEnterPlanModeAsk
                      ? t("planMode.enterDialog.header")
                      : localizedText(q.header, t)}
                  </span>
                )}
                {q.multiSelect && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground font-normal">
                    {t("planMode.question.multiSelect", { defaultValue: "multi" })}
                  </span>
                )}
              </div>

              {isFreeText ? (
                <div className="pl-5">
                  {q.inputKind === "textarea" ? (
                    <Textarea
                      placeholder={localizedText(q.text, t)}
                      value={state?.customInput || ""}
                      onChange={(e) => setCustomInput(q.questionId, e.target.value)}
                      rows={3}
                      className="text-sm resize-none"
                    />
                  ) : (
                    <Input
                      placeholder={localizedText(q.text, t)}
                      value={state?.customInput || ""}
                      onChange={(e) => setCustomInput(q.questionId, e.target.value)}
                      className="text-sm h-9"
                    />
                  )}
                </div>
              ) : (
              <div className="pl-5 space-y-1.5">
                {isDirectionCards ? (
                  <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    {q.options.map((opt) => (
                      <DirectionCardView
                        key={opt.value}
                        option={opt}
                        selected={state?.selected.has(opt.value) ?? false}
                        onSelect={() => toggleOption(q.questionId, opt.value, q.multiSelect)}
                      />
                    ))}
                  </div>
                ) : (
                  <>
                {q.options.map((opt) => {
                  const isSelected = state?.selected.has(opt.value) ?? false
                  const isDefault = q.defaultValues?.includes(opt.value) ?? false
                  return (
                    <button
                      key={opt.value}
                      type="button"
                      aria-pressed={isSelected}
                      onClick={() => toggleOption(q.questionId, opt.value, q.multiSelect)}
                      onMouseEnter={
                        hasQuestionPreview
                          ? () =>
                              setFocusedOption((prev) => ({
                                ...prev,
                                [q.questionId]: opt.value,
                              }))
                          : undefined
                      }
                      className={cn(
                        "w-full text-left px-3 py-2 rounded-md border text-sm transition-colors cursor-pointer",
                        isSelected
                          ? "border-border bg-secondary text-foreground"
                          : opt.recommended
                            ? "border-amber-500/40 bg-amber-500/5 hover:bg-amber-500/10"
                            : "border-border hover:bg-secondary/40",
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "h-4 w-4 border-2 flex items-center justify-center shrink-0",
                            q.multiSelect ? "rounded-sm" : "rounded-full",
                            isSelected
                              ? "border-transparent bg-blue-500"
                              : "border-muted-foreground/30",
                          )}
                        >
                          {isSelected && <Check className="h-2.5 w-2.5 text-white" />}
                        </div>
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-1.5 flex-wrap">
                            <span className="font-medium">
                              {isEnterPlanModeAsk
                                ? t(`planMode.enterDialog.option.${opt.value}.label`, {
                                    defaultValue: fallbackText(opt.label),
                                  })
                                : localizedText(opt.label, t)}
                            </span>
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
                          {(() => {
                            const desc = isEnterPlanModeAsk
                              ? t(`planMode.enterDialog.option.${opt.value}.description`, {
                                  defaultValue: fallbackText(opt.description),
                                })
                              : localizedText(opt.description, t)
                            return desc ? (
                              <div className="text-xs text-muted-foreground mt-0.5">{desc}</div>
                            ) : null
                          })()}
                          {/* Inline preview for narrow viewports where the
                              side pane (`hidden md:block`) is not shown. */}
                          {opt.preview && (
                            <div className="md:hidden">
                              <OptionPreview option={opt} />
                            </div>
                          )}
                        </div>
                      </div>
                    </button>
                  )
                })}
                  </>
                )}

                {/* Custom input is gated behind an explicit "Other" choice so
                    regular selections and free-form answers don't blur together. */}
                {q.allowCustom && (
                  <>
                    <button
                      type="button"
                      aria-pressed={customSelected}
                      onClick={() => toggleCustomOption(q.questionId, q.multiSelect)}
                      onMouseEnter={
                        hasQuestionPreview
                          ? () =>
                              setFocusedOption((prev) => ({
                                ...prev,
                                [q.questionId]: CUSTOM_OPTION_FOCUS,
                              }))
                          : undefined
                      }
                      className={cn(
                        "w-full text-left px-3 py-2 rounded-md border text-sm transition-colors cursor-pointer",
                        customSelected
                          ? "border-border bg-secondary text-foreground"
                          : "border-border hover:bg-secondary/40",
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "h-4 w-4 border-2 flex items-center justify-center shrink-0",
                            q.multiSelect ? "rounded-sm" : "rounded-full",
                            customSelected
                              ? "border-transparent bg-blue-500"
                              : "border-muted-foreground/30",
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
              )}
            </div>

            {/* Right column: side preview pane. Absolutely filled inside the
                grid cell so its content height never drives the row height —
                hovering between previews of different sizes cannot reflow the
                layout (issue #433); tall previews scroll internally. */}
            {previewOpt && (
              <div className="hidden md:block relative">
                <div className="absolute inset-0 flex flex-col">
                  <div className="flex items-center text-[10px] uppercase tracking-wide text-muted-foreground mb-1 leading-5 h-5 shrink-0">
                    {t("planMode.question.preview", { defaultValue: "Preview" })}:{" "}
                    {localizedText(previewOpt.label, t)}
                  </div>
                  <div
                    className={cn(
                      "flex-1 min-h-0 flex flex-col transition-opacity",
                      previewIsFallback && "opacity-60",
                    )}
                  >
                    <OptionPreview option={previewOpt} fill />
                  </div>
                </div>
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
          type="button"
          size="sm"
          onClick={handleSubmit}
          disabled={submitting || timedOut}
          className={cn(
            "gap-1.5",
            error && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
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
