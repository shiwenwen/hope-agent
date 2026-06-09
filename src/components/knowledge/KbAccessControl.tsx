import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"

import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { KbAccess } from "@/types/knowledge"

/** Off (not attached) | read | write. */
export type KbAccessValue = "off" | KbAccess

/**
 * A compact, always-visible 3-state segmented control for a knowledge space's
 * access level: 关闭 / 只读 / 读写 (Off / Read / Read+Write). Replaces the old
 * tiny read/write chip + on/off Switch, which made the read-vs-write distinction
 * effectively invisible. Used by the composer KnowledgePicker and the project
 * settings KB section so both surfaces read/write the same way.
 *
 * `allowWrite=false` hides the write segment (external read-only vaults are
 * capped to read, D11). `disabled` renders the current state read-only (e.g. a
 * project-managed attach that must be changed at the project level).
 */
export function KbAccessControl({
  value,
  allowWrite,
  disabled = false,
  busy = false,
  onChange,
  className,
}: {
  value: KbAccessValue
  allowWrite: boolean
  disabled?: boolean
  busy?: boolean
  onChange: (next: KbAccessValue) => void
  className?: string
}) {
  const { t } = useTranslation()
  // Always surface the write segment when the value already IS write (e.g. a
  // project-granted write on an otherwise read-capped row) so the state reads true.
  const showWrite = allowWrite || value === "write"
  const segs: { v: KbAccessValue; label: string; tip: string }[] = [
    { v: "off", label: t("knowledge.picker.accessOff"), tip: t("knowledge.picker.accessOffTip") },
    { v: "read", label: t("knowledge.picker.accessRead"), tip: t("knowledge.picker.accessReadTip") },
    ...(showWrite
      ? [
          {
            v: "write" as KbAccessValue,
            label: t("knowledge.picker.accessWrite"),
            tip: t("knowledge.picker.accessWriteTip"),
          },
        ]
      : []),
  ]
  return (
    <div
      role="radiogroup"
      className={cn(
        "inline-flex shrink-0 items-center rounded-lg border border-border/50 bg-secondary/30 p-0.5",
        className,
      )}
    >
      {busy && <Loader2 className="mx-1 h-3 w-3 animate-spin text-muted-foreground" />}
      {segs.map((s) => {
        const active = value === s.v
        return (
          <IconTip key={s.v} label={s.tip}>
            <button
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled || busy}
              onClick={() => {
                if (!active) onChange(s.v)
              }}
              className={cn(
                "rounded-md px-2 py-0.5 text-[10px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60",
                active
                  ? "bg-primary text-primary-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-secondary hover:text-foreground",
              )}
            >
              {s.label}
            </button>
          </IconTip>
        )
      })}
    </div>
  )
}
