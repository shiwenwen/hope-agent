import { useEffect, useRef } from "react"
import { ArrowUpRight, CircleAlert, CircleCheck, CircleEllipsis, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { PetActivity, PetActivityStatus } from "@/types/pet"

const MAX_VISIBLE = 8

function StatusIcon({ status }: { status: PetActivityStatus }) {
  if (status === "needs_input" || status === "blocked") {
    return <CircleAlert className="h-4 w-4" aria-hidden="true" />
  }
  if (status === "ready") return <CircleCheck className="h-4 w-4" aria-hidden="true" />
  return <CircleEllipsis className="h-4 w-4" aria-hidden="true" />
}

interface PetActivityTrayProps {
  activities: PetActivity[]
  total: number
  stale: boolean
  onOpen: (activity: PetActivity) => void
  onOpenAll: () => void
  onClose: () => void
  measuring?: boolean
}

export function PetActivityTray({
  activities,
  total,
  stale,
  onOpen,
  onOpenAll,
  onClose,
  measuring,
}: PetActivityTrayProps) {
  const { t, i18n } = useTranslation()
  const visible = activities.slice(0, MAX_VISIBLE)
  const remaining = Math.max(0, total - visible.length)
  const listRef = useRef<HTMLUListElement>(null)

  useEffect(() => {
    if (!measuring) listRef.current?.querySelector<HTMLButtonElement>("button")?.focus()
  }, [measuring])
  const activityTitle = (activity: PetActivity) => {
    if (activity.titleKind === "incognito") {
      return t("pet.tray.incognitoTitle", { defaultValue: "Incognito conversation" })
    }
    if (activity.titleKind === "untitled") {
      return t("pet.tray.untitledTitle", { defaultValue: "New conversation" })
    }
    return activity.title || t("pet.tray.untitledTitle", { defaultValue: "New conversation" })
  }

  return (
    <section
      aria-label={t("pet.tray.title", { defaultValue: "Conversation activity" })}
      aria-hidden={measuring || undefined}
      className="w-[380px] overflow-hidden rounded-2xl border border-border/80 bg-popover/95 text-popover-foreground shadow-2xl backdrop-blur-md"
    >
      <header className="flex items-center justify-between gap-2 border-b border-border/70 px-3 py-2.5">
        <div className="min-w-0">
          <h2 className="truncate text-sm font-semibold">
            {t("pet.tray.title", { defaultValue: "Conversation activity" })}
          </h2>
          <p className="text-xs text-muted-foreground">
            {stale
              ? t("pet.tray.reconnecting", { defaultValue: "Refreshing status…" })
              : t("pet.tray.count", { count: total, defaultValue: `${total} active` })}
          </p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled={measuring}
          onClick={onClose}
          aria-label={t("pet.tray.close", { defaultValue: "Close activity" })}
          className="h-7 w-7"
        >
          <X className="h-3.5 w-3.5" aria-hidden="true" />
        </Button>
      </header>
      <ul
        ref={listRef}
        className="max-h-[360px] overflow-y-auto p-2"
        onKeyDown={(event) => {
          if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return
          const buttons = Array.from(
            listRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
          )
          if (buttons.length === 0) return
          event.preventDefault()
          const current = buttons.indexOf(document.activeElement as HTMLButtonElement)
          const next =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? buttons.length - 1
                : event.key === "ArrowDown"
                  ? (current + 1 + buttons.length) % buttons.length
                  : (current - 1 + buttons.length) % buttons.length
          buttons[next]?.focus()
        }}
      >
        {visible.map((activity) => (
          <li key={`${activity.activityId}:${activity.boundary ?? 0}`}>
            <Button
              type="button"
              variant="ghost"
              disabled={measuring}
              onClick={() => onOpen(activity)}
              className="group h-auto w-full justify-start gap-2 rounded-xl px-2.5 py-2 text-left hover:bg-accent"
            >
              <span
                className={cn(
                  "mt-0.5 shrink-0",
                  activity.status === "needs_input" && "text-amber-600 dark:text-amber-400",
                  activity.status === "blocked" && "text-destructive",
                  activity.status === "ready" && "text-emerald-600 dark:text-emerald-400",
                  activity.status === "running" && "text-sky-600 dark:text-sky-400",
                )}
              >
                <StatusIcon status={activity.status} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">
                  {activityTitle(activity)}
                </span>
                <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                  {activity.agentId ? `${activity.agentId} · ` : ""}
                  {new Intl.DateTimeFormat(i18n.language, {
                    hour: "2-digit",
                    minute: "2-digit",
                  }).format(new Date(activity.updatedAt))}
                </span>
              </span>
              <ArrowUpRight
                className="mt-1 h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
                aria-hidden="true"
              />
            </Button>
          </li>
        ))}
      </ul>
      {remaining > 0 && (
        <div className="border-t border-border/70 p-2">
          <Button
            type="button"
            variant="ghost"
            disabled={measuring}
            onClick={onOpenAll}
            className="h-8 w-full justify-center text-xs"
          >
            {t("pet.tray.openRemaining", {
              count: remaining,
              defaultValue: `View ${remaining} more in Hope`,
            })}
          </Button>
        </div>
      )}
    </section>
  )
}
