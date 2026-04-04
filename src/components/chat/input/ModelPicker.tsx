import { useState, useRef, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Brain, ChevronRight } from "lucide-react"
import type { AvailableModel, ActiveModel } from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"

interface ModelPickerProps {
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  onModelChange: (key: string) => void
  onEffortChange: (effort: string) => void
  currentModelInfo?: AvailableModel
}

export default function ModelPicker({
  availableModels,
  activeModel,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  currentModelInfo,
}: ModelPickerProps) {
  const { t } = useTranslation()

  const [showModelMenu, setShowModelMenu] = useState(false)
  const [menuProvider, setMenuProvider] = useState<string | null>(null)
  const modelMenuRef = useRef<HTMLDivElement>(null)
  const [showThinkMenu, setShowThinkMenu] = useState(false)
  const thinkMenuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (modelMenuRef.current && !modelMenuRef.current.contains(e.target as Node)) {
        setShowModelMenu(false)
        setMenuProvider(null)
      }
      if (thinkMenuRef.current && !thinkMenuRef.current.contains(e.target as Node)) {
        setShowThinkMenu(false)
      }
    }
    if (showModelMenu || showThinkMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showModelMenu, showThinkMenu])

  return (
    <>

      {availableModels.length > 0 && (
        <div className="relative" ref={modelMenuRef}>
          <button
            onClick={() => {
              setShowModelMenu(!showModelMenu)
              setMenuProvider(null)
            }}
            className="flex items-center gap-1 bg-transparent text-muted-foreground hover:text-foreground text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 max-w-[200px]"
          >
            <span className="truncate">
              {currentModelInfo
                ? `${currentModelInfo.providerName} / ${currentModelInfo.modelName}`
                : t("chat.selectModel")}
            </span>
          </button>


          {showModelMenu && (
            <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[160px] max-w-[220px] p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
              <div className="flex flex-col gap-0.5">
                {Array.from(
                  new Map(availableModels.map((m) => [m.providerId, m.providerName])),
                ).map(([pid, pname]) => {
                  const models = availableModels.filter((m) => m.providerId === pid)
                  const hasMultiple = models.length > 1
                  return (
                    <div key={pid} className="relative">
                      <button
                        className={cn(
                          "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150 flex items-center justify-between gap-3",
                          menuProvider === pid
                            ? "bg-secondary text-foreground shadow-sm"
                            : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                        )}
                        onMouseEnter={() => setMenuProvider(hasMultiple ? pid : null)}
                        onClick={() => {
                          if (!hasMultiple) {
                            onModelChange(`${models[0].providerId}::${models[0].modelId}`)
                            setShowModelMenu(false)
                            setMenuProvider(null)
                          }
                        }}
                      >
                        <span className="truncate">{pname}</span>
                        {hasMultiple && (
                          <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-50" />
                        )}
                      </button>

                      {/* Submenu */}
                      {hasMultiple && menuProvider === pid && (
                        <div className="absolute left-full bottom-[-6px] ml-1.5 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[160px] max-w-[260px] p-1.5">
                          <div className="flex flex-col gap-0.5 max-h-[50vh] overflow-y-auto overscroll-contain">
                            {models.map((m) => (
                              <button
                                key={m.modelId}
                                className={cn(
                                  "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150 truncate",
                                  activeModel?.providerId === m.providerId &&
                                    activeModel?.modelId === m.modelId
                                    ? "bg-secondary text-foreground font-medium shadow-sm"
                                    : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                                )}
                                onClick={() => {
                                  onModelChange(`${m.providerId}::${m.modelId}`)
                                  setShowModelMenu(false)
                                  setMenuProvider(null)
                                }}
                              >
                                {m.modelName}
                              </button>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>
      )}


      {(currentModelInfo?.reasoning ?? true) && (
        <div className="relative" ref={thinkMenuRef}>
          <button
            onClick={() => setShowThinkMenu(!showThinkMenu)}
            className="flex items-center gap-1 bg-transparent text-muted-foreground hover:text-foreground text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap"
          >
            <Brain className="h-3.5 w-3.5 shrink-0" />
            <span>
              {getEffortOptionsForType(currentModelInfo?.apiType, t).find(
                (o) => o.value === reasoningEffort,
              )?.label ?? reasoningEffort}
            </span>
          </button>

          {showThinkMenu && (
            <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[120px] p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
              <div className="flex flex-col gap-0.5">
                {getEffortOptionsForType(currentModelInfo?.apiType, t).map((opt) => (
                  <button
                    key={opt.value}
                    className={cn(
                      "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150",
                      reasoningEffort === opt.value
                        ? "bg-secondary text-foreground font-medium shadow-sm"
                        : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                    )}
                    onClick={() => {
                      onEffortChange(opt.value)
                      setShowThinkMenu(false)
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </>
  )
}
