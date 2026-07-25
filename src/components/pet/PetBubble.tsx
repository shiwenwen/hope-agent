import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react"
import { Check, CircleAlert, CircleX, LoaderCircle, Reply, Send, Square, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import type { PetActivity, PetActivityStatus } from "@/types/pet"
import { petPreviewText } from "./petPreviewText"

export type PetReplyDispatch = "queued" | "sent"

interface PetBubbleProps {
  activity: PetActivity
  expanded: boolean
  interactionPending: boolean
  livePreview?: string
  onOpen: () => void
  onDismiss: () => void
  onExpandReply: () => void
  onCollapseReply: () => void
  onReply: (text: string) => Promise<PetReplyDispatch>
  onStop: () => Promise<void>
  onViewed?: () => void
  measuring?: boolean
  nativeHovered?: boolean
  nativeHoveredAction?: "dismiss" | "reply" | "stop" | null
}

const TITLE_MAX_DISPLAY_UNITS = 21
const VIEW_DWELL_MS = 700

function titleDisplayUnits(character: string): number {
  return character.codePointAt(0)! > 0xff ? 2 : 1
}

function compactTitle(value: string): string {
  const characters = Array.from(value)
  const totalUnits = characters.reduce(
    (total, character) => total + titleDisplayUnits(character),
    0,
  )
  if (totalUnits <= TITLE_MAX_DISPLAY_UNITS) return value

  const contentBudget = TITLE_MAX_DISPLAY_UNITS - 2
  const leadingBudget = Math.ceil(contentBudget * 0.6)
  const trailingBudget = contentBudget - leadingBudget
  let leading = ""
  let leadingUnits = 0
  for (const character of characters) {
    const units = titleDisplayUnits(character)
    if (leadingUnits + units > leadingBudget) break
    leading += character
    leadingUnits += units
  }
  let trailing = ""
  let trailingUnits = 0
  for (let index = characters.length - 1; index >= 0; index -= 1) {
    const character = characters[index]
    const units = titleDisplayUnits(character)
    if (trailingUnits + units > trailingBudget) break
    trailing = character + trailing
    trailingUnits += units
  }
  return `${leading}…${trailing}`
}

function StatusIndicator({ status }: { status: PetActivityStatus }) {
  if (status === "running") {
    return (
      <span className="flex h-7 w-7 items-center justify-center rounded-full text-muted-foreground">
        <LoaderCircle className="h-5 w-5 animate-spin motion-reduce:animate-none" />
      </span>
    )
  }
  if (status === "ready") {
    return (
      <span className="flex h-7 w-7 items-center justify-center rounded-full bg-emerald-500/15 text-emerald-600 dark:text-emerald-400">
        <Check className="h-4 w-4" />
      </span>
    )
  }
  if (status === "blocked") {
    return (
      <span className="flex h-7 w-7 items-center justify-center rounded-full bg-destructive/15 text-destructive">
        <CircleX className="h-4 w-4" />
      </span>
    )
  }
  return (
    <span className="flex h-7 w-7 items-center justify-center rounded-full bg-amber-500/15 text-amber-600 dark:text-amber-400">
      <CircleAlert className="h-4 w-4" />
    </span>
  )
}

export function PetBubble({
  activity,
  expanded,
  interactionPending,
  livePreview,
  onOpen,
  onDismiss,
  onExpandReply,
  onCollapseReply,
  onReply,
  onStop,
  onViewed,
  measuring,
  nativeHovered = false,
  nativeHoveredAction = null,
}: PetBubbleProps) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState("")
  const [submitting, setSubmitting] = useState(false)
  const [stopping, setStopping] = useState(false)
  const [feedback, setFeedback] = useState<PetReplyDispatch | "error" | null>(null)
  const [pointerHovered, setPointerHovered] = useState(false)
  const viewStartedAtRef = useRef<number | null>(null)
  const viewReportedRef = useRef(false)
  const title =
    activity.titleKind === "incognito"
      ? t("pet.tray.incognitoTitle", { defaultValue: "Incognito conversation" })
      : activity.titleKind === "untitled"
        ? t("pet.tray.untitledTitle", { defaultValue: "New conversation" })
        : activity.title || t("pet.tray.untitledTitle", { defaultValue: "New conversation" })
  const displayTitle = compactTitle(title)
  const fallbackSummary =
    activity.status === "running"
      ? t("pet.bubble.running", { defaultValue: "Thinking" })
      : activity.status === "needs_input"
        ? t("pet.bubble.needsInput", { count: 1, defaultValue: "Needs your input" })
        : activity.status === "blocked"
          ? t("pet.bubble.blocked", { count: 1, defaultValue: "Conversation blocked" })
          : t("pet.bubble.ready", { count: 1, defaultValue: "Conversation ready" })
  const markdownSummary =
    activity.status === "running"
      ? livePreview || fallbackSummary
      : activity.status === "ready"
        ? activity.preview || fallbackSummary
        : fallbackSummary
  const summary = petPreviewText(markdownSummary) || fallbackSummary
  const viewBoundary =
    activity.boundary && ["ready", "blocked"].includes(activity.status)
      ? `${activity.activityId}:${activity.boundary}`
      : null
  const viewHovered = pointerHovered || nativeHovered

  useEffect(() => {
    viewStartedAtRef.current = null
    viewReportedRef.current = false
  }, [viewBoundary])

  useEffect(() => {
    if (measuring || !viewBoundary || viewReportedRef.current) {
      viewStartedAtRef.current = null
      return
    }
    if (viewHovered) {
      viewStartedAtRef.current ??= Date.now()
      return
    }
    const startedAt = viewStartedAtRef.current
    viewStartedAtRef.current = null
    if (startedAt === null || Date.now() - startedAt < VIEW_DWELL_MS) return
    viewReportedRef.current = true
    onViewed?.()
  }, [measuring, onViewed, viewBoundary, viewHovered])

  const submit = async (event?: FormEvent) => {
    event?.preventDefault()
    const text = draft.trim()
    if (!text || submitting || measuring) return
    setSubmitting(true)
    setFeedback(null)
    try {
      const outcome = await onReply(text)
      setDraft("")
      setFeedback(outcome)
    } catch {
      setFeedback("error")
    } finally {
      setSubmitting(false)
    }
  }

  const handleComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return
    event.preventDefault()
    void submit()
  }

  const stop = async () => {
    if (stopping || measuring) return
    setStopping(true)
    try {
      await onStop()
    } finally {
      setStopping(false)
    }
  }

  const showReplyAction = !interactionPending && !expanded
  const showStopAction = activity.status === "running" && !expanded
  const showHoverActions = showReplyAction || showStopAction

  return (
    <div
      className="group relative mx-auto w-[344px]"
      data-pet-activity-id={activity.activityId}
      aria-hidden={measuring || undefined}
      onPointerEnter={() => {
        if (!measuring) setPointerHovered(true)
      }}
      onPointerLeave={() => {
        if (!measuring) setPointerHovered(false)
      }}
    >
      <Button
        type="button"
        variant="ghost"
        size="icon"
        disabled={measuring}
        onClick={onDismiss}
        aria-label={t("pet.bubble.dismiss", { defaultValue: "Dismiss" })}
        data-pet-hover-action="dismiss"
        className={cn(
          "pointer-events-none absolute -left-2 -top-2 z-20 h-6 w-6 rounded-full border border-border/50 bg-popover/70 text-muted-foreground/70 opacity-0 shadow-none backdrop-blur-md transition-[background-color,opacity] hover:bg-muted/80 hover:text-muted-foreground/70 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100",
          nativeHovered && "pointer-events-auto opacity-100",
          nativeHoveredAction === "dismiss" && "bg-muted/80",
        )}
      >
        <X className="h-3.5 w-3.5" />
      </Button>
      <section
        aria-label={title}
        className="w-full overflow-hidden rounded-[26px] border border-white/55 bg-popover/75 text-popover-foreground shadow-[0_8px_28px_rgb(0_0_0/0.12)] backdrop-blur-xl backdrop-saturate-150 dark:border-white/10 dark:bg-popover/72 dark:shadow-[0_10px_30px_rgb(0_0_0/0.3)]"
      >
        <div className="relative flex min-h-[52px] items-center gap-1.5 px-3 py-1.5">
          <Button
            type="button"
            variant="ghost"
            disabled={measuring}
            onClick={onOpen}
            className={cn(
              "h-auto min-w-0 flex-1 justify-start whitespace-normal rounded-xl p-0 text-left transition-[margin] hover:bg-transparent",
              showStopAction &&
                "group-hover:mr-8 group-focus-within:mr-8 motion-reduce:transition-none",
              showStopAction && nativeHovered && "mr-8",
            )}
          >
            <span className="line-clamp-2 min-w-0 flex-1 text-left text-xs leading-4 text-muted-foreground">
              <span className="inline-block max-w-[52%] truncate align-bottom text-[13px] font-semibold text-popover-foreground">
                {displayTitle}
              </span>
              <span
                className="mx-1 inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-popover-foreground/85 align-middle"
                aria-hidden="true"
                data-pet-title-separator
              />
              <span
                className={cn(
                  "text-xs text-muted-foreground",
                  activity.status === "running" && "animate-text-shimmer",
                )}
              >
                {summary}
              </span>
            </span>
          </Button>
          <div className="relative h-7 w-7 shrink-0">
            <span
              className={cn(
                "absolute inset-0 transition-opacity",
                showHoverActions &&
                  "group-hover:opacity-0 group-focus-within:opacity-0 motion-reduce:transition-none",
                showHoverActions && nativeHovered && "opacity-0",
              )}
            >
              <StatusIndicator status={activity.status} />
            </span>
            {showHoverActions && (
              <span
                className={cn(
                  "pointer-events-none absolute right-0 top-0 flex gap-1 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 motion-reduce:transition-none",
                  nativeHovered && "pointer-events-auto opacity-100",
                )}
              >
                {showReplyAction && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    disabled={measuring}
                    onClick={onExpandReply}
                    aria-label={t("pet.bubble.reply", { defaultValue: "Reply" })}
                    data-pet-hover-action="reply"
                    className={cn(
                      "h-7 w-7 shrink-0 rounded-full bg-muted/40 text-muted-foreground/70 shadow-none transition-colors hover:bg-muted hover:text-muted-foreground/70",
                      nativeHoveredAction === "reply" && "bg-muted",
                    )}
                  >
                    <Reply className="h-3.5 w-3.5" strokeWidth={1.75} />
                  </Button>
                )}
                {showStopAction && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    disabled={stopping || measuring}
                    onClick={() => void stop()}
                    aria-label={t("chat.stopReply", { defaultValue: "Stop reply" })}
                    data-pet-hover-action="stop"
                    className={cn(
                      "h-7 w-7 shrink-0 rounded-full bg-muted/40 text-muted-foreground/70 shadow-none transition-colors hover:bg-muted hover:text-muted-foreground/70",
                      nativeHoveredAction === "stop" && "bg-muted",
                    )}
                  >
                    {stopping ? (
                      <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                    ) : (
                      <Square className="h-3 w-3 fill-current" strokeWidth={1.75} />
                    )}
                  </Button>
                )}
              </span>
            )}
          </div>
        </div>
        {expanded && !interactionPending && (
          <form onSubmit={(event) => void submit(event)} className="border-t border-border/70 p-3">
            <div className="flex items-end gap-1.5">
              <Textarea
                autoFocus={!measuring}
                surface="embedded"
                rows={2}
                value={draft}
                disabled={submitting || measuring}
                onChange={(event) => {
                  setDraft(event.target.value)
                  if (feedback) setFeedback(null)
                }}
                onKeyDown={handleComposerKeyDown}
                placeholder={t("quickChat.placeholder", { defaultValue: "Ask anything…" })}
                className="min-h-[52px] resize-none rounded-xl"
              />
              <Button
                type="submit"
                size="icon"
                disabled={!draft.trim() || submitting || measuring}
                aria-label={t("chat.send", { defaultValue: "Send" })}
                className="h-7 w-7 shrink-0 rounded-full"
              >
                <Send className="h-3.5 w-3.5" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                disabled={submitting || measuring}
                onClick={onCollapseReply}
                aria-label={t("common.close", { defaultValue: "Close" })}
                className="h-7 w-7 shrink-0 rounded-full"
              >
                <X className="h-3.5 w-3.5" />
              </Button>
            </div>
            {feedback && (
              <p
                className={cn(
                  "mt-1.5 px-1 text-[11px]",
                  feedback === "error" ? "text-destructive" : "text-muted-foreground",
                )}
              >
                {feedback === "queued"
                  ? t("pet.bubble.queued", { defaultValue: "Reply queued" })
                  : feedback === "sent"
                    ? t("pet.bubble.sent", { defaultValue: "Reply sent" })
                    : t("pet.bubble.sendFailed", { defaultValue: "Could not send reply" })}
              </p>
            )}
          </form>
        )}
      </section>
    </div>
  )
}
