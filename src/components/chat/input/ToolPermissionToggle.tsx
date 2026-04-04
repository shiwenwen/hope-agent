import { useState, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { useClickOutside } from "@/hooks/useClickOutside"
import { cn } from "@/lib/utils"
import { Shield, ShieldCheck, ShieldAlert } from "lucide-react"
import type { ToolPermissionMode } from "@/types/chat"

interface ToolPermissionToggleProps {
  toolPermissionMode: ToolPermissionMode
  onToolPermissionChange: (mode: ToolPermissionMode) => void
}

export default function ToolPermissionToggle({
  toolPermissionMode,
  onToolPermissionChange,
}: ToolPermissionToggleProps) {
  const { t } = useTranslation()
  const [showPermMenu, setShowPermMenu] = useState(false)
  const permMenuRef = useRef<HTMLDivElement>(null)

  useClickOutside(permMenuRef, useCallback(() => setShowPermMenu(false), []))

  return (
    <div className="relative" ref={permMenuRef}>
      <button
        onClick={() => setShowPermMenu(!showPermMenu)}
        className={cn(
          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
          toolPermissionMode === "full_approve"
            ? "text-destructive"
            : toolPermissionMode === "ask_every_time"
              ? "text-amber-600 dark:text-amber-400"
              : "text-muted-foreground hover:text-foreground"
        )}
      >
        {toolPermissionMode === "full_approve" ? (
          <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
        ) : toolPermissionMode === "ask_every_time" ? (
          <ShieldCheck className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <Shield className="h-3.5 w-3.5 shrink-0" />
        )}
        <span>
          {toolPermissionMode === "full_approve"
            ? t("chat.toolPermissionFull")
            : toolPermissionMode === "ask_every_time"
              ? t("chat.toolPermissionAsk")
              : t("chat.toolPermissionAuto")}
        </span>
      </button>

      {showPermMenu && (
        <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[180px] p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
          <div className="flex flex-col gap-0.5">
            {([
              { value: "auto" as const, label: t("chat.toolPermissionAuto"), desc: t("chat.toolPermissionAutoDesc"), icon: Shield },
              { value: "ask_every_time" as const, label: t("chat.toolPermissionAsk"), desc: t("chat.toolPermissionAskDesc"), icon: ShieldCheck },
              { value: "full_approve" as const, label: t("chat.toolPermissionFull"), desc: t("chat.toolPermissionFullDesc"), icon: ShieldAlert },
            ]).map((opt) => (
              <button
                key={opt.value}
                className={cn(
                  "w-full text-left px-2.5 py-2 rounded-md transition-all duration-150 flex items-start gap-2",
                  toolPermissionMode === opt.value
                    ? "bg-secondary text-foreground font-medium shadow-sm"
                    : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                )}
                onClick={() => {
                  onToolPermissionChange(opt.value)
                  setShowPermMenu(false)
                }}
              >
                <opt.icon className={cn(
                  "h-3.5 w-3.5 mt-0.5 shrink-0",
                  opt.value === "full_approve" && "text-destructive",
                  opt.value === "ask_every_time" && "text-amber-600 dark:text-amber-400",
                )} />
                <div className="flex flex-col">
                  <span className="text-[13px]">{opt.label}</span>
                  <span className="text-[11px] text-muted-foreground font-normal">{opt.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
