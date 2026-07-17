import { useState } from "react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { formatRemaining, useCountdownRemainingSec, useNowMs } from "@/lib/countdown"
import type { SessionMeta } from "@/types/chat"

const SIZE = 12
const STROKE = 2
const RADIUS = (SIZE - STROKE) / 2
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

interface PendingCountdownRingProps {
  countdown: NonNullable<SessionMeta["pendingCountdown"]>
}

/**
 * 12px amber ring that drains as the earliest pending interaction approaches
 * its auto-resolve deadline. Numbers only appear in the hover tooltip; the
 * ring itself stays text-free so it never disturbs the row layout. Once the
 * deadline passes the ring sits empty until the backend timeout event
 * refreshes the session list (no optimistic local state flip).
 */
export default function PendingCountdownRing({ countdown }: PendingCountdownRingProps) {
  const { t } = useTranslation()
  // Clock-skew correction, same intent as useApprovals: trust the server's
  // deadline, translate it onto the local clock. Skew is anchored at mount
  // against the shared cached clock (never a raw `Date.now()` in render). The
  // parent keys this component by `${deadlineAtMs}:${serverNowMs}`, so a new
  // snapshot (e.g. a corrected server clock, or the next-earliest deadline)
  // remounts and re-anchors instead of drifting on a stale skew.
  const nowMs = useNowMs()
  const [skewMs] = useState(() => countdown.serverNowMs - nowMs)
  const localDeadlineMs = countdown.deadlineAtMs - skewMs
  const remaining = useCountdownRemainingSec(localDeadlineMs) ?? 0
  const ratio = Math.min(1, Math.max(0, (remaining * 1000) / countdown.totalMs))

  return (
    <IconTip label={t("chat.pendingCountdownTooltip", { time: formatRemaining(remaining) })}>
      <span className="inline-flex h-3 w-3 shrink-0 cursor-default items-center justify-center">
        <svg
          width={SIZE}
          height={SIZE}
          viewBox={`0 0 ${SIZE} ${SIZE}`}
          className="-rotate-90"
          aria-hidden="true"
        >
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={RADIUS}
            fill="none"
            strokeWidth={STROKE}
            className="stroke-amber-500/25"
          />
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={RADIUS}
            fill="none"
            strokeWidth={STROKE}
            strokeLinecap="round"
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - ratio)}
            className="stroke-amber-500 transition-[stroke-dashoffset] duration-1000 ease-linear"
          />
        </svg>
      </span>
    </IconTip>
  )
}
