import { useState, useRef, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { Send, Square, Brain, ChevronRight, ImagePlus, Paperclip, X, Slash, Shield, ShieldCheck, ShieldAlert, ClipboardList } from "lucide-react"
import type { AvailableModel, ActiveModel, ToolPermissionMode } from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"
import { useSlashCommands, type SlashCommandActions } from "./slash-commands/useSlashCommands"
import { useLightbox } from "@/components/common/ImageLightbox"
import SlashCommandMenu from "./slash-commands/SlashCommandMenu"
import type { CommandResult } from "./slash-commands/types"

interface ChatInputProps {
  input: string
  onInputChange: (value: string) => void
  onSend: () => void
  loading: boolean
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  onModelChange: (key: string) => void
  onEffortChange: (effort: string) => void
  attachedFiles: File[]
  onAttachFiles: (files: File[]) => void
  onRemoveFile: (index: number) => void
  pendingMessage?: string | null
  onCancelPending?: () => void
  onStop?: () => void
  // Slash command support
  currentSessionId?: string | null
  currentAgentId?: string
  onCommandAction?: (result: CommandResult) => void
  // Tool permission mode
  toolPermissionMode: ToolPermissionMode
  onToolPermissionChange: (mode: ToolPermissionMode) => void
  // Plan mode
  planState?: "off" | "planning" | "executing"
  planProgress?: number
  onEnterPlanMode?: () => void
  onExitPlanMode?: () => void
  onTogglePlanPanel?: () => void
}

export default function ChatInput({
  input,
  onInputChange,
  onSend,
  loading,
  availableModels,
  activeModel,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  attachedFiles,
  onAttachFiles,
  onRemoveFile,
  pendingMessage,
  onCancelPending,
  onStop,
  currentSessionId,
  currentAgentId = "default",
  onCommandAction,
  toolPermissionMode,
  onToolPermissionChange,
  planState = "off",
  planProgress = 0,
  onEnterPlanMode,
  onExitPlanMode,
  onTogglePlanPanel,
}: ChatInputProps) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const imageInputRef = useRef<HTMLInputElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
  }
  const slash = useSlashCommands(input, onInputChange, slashActions)

  // Model selector popup state
  const [showModelMenu, setShowModelMenu] = useState(false)
  const [menuProvider, setMenuProvider] = useState<string | null>(null)
  const modelMenuRef = useRef<HTMLDivElement>(null)
  const [showThinkMenu, setShowThinkMenu] = useState(false)
  const thinkMenuRef = useRef<HTMLDivElement>(null)
  const [showPermMenu, setShowPermMenu] = useState(false)
  const permMenuRef = useRef<HTMLDivElement>(null)

  // Close menus on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (modelMenuRef.current && !modelMenuRef.current.contains(e.target as Node)) {
        setShowModelMenu(false)
        setMenuProvider(null)
      }
      if (thinkMenuRef.current && !thinkMenuRef.current.contains(e.target as Node)) {
        setShowThinkMenu(false)
      }
      if (permMenuRef.current && !permMenuRef.current.contains(e.target as Node)) {
        setShowPermMenu(false)
      }
    }
    if (showModelMenu || showThinkMenu || showPermMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showModelMenu, showThinkMenu, showPermMenu])

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files
      if (files) {
        onAttachFiles(Array.from(files))
      }
      e.target.value = ""
    },
    [onAttachFiles],
  )

  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items
      if (!items) return
      const files: File[] = []
      for (let i = 0; i < items.length; i++) {
        const item = items[i]
        if (item.kind === "file") {
          const file = item.getAsFile()
          if (file) files.push(file)
        }
      }
      if (files.length > 0) {
        e.preventDefault()
        onAttachFiles(files)
      }
    },
    [onAttachFiles],
  )

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    // Let slash command menu handle keys first
    if (slash.handleKeyDown(e)) return
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      onSend()
    }
  }

  const currentModelInfo = availableModels.find(
    (m) => m.providerId === activeModel?.providerId && m.modelId === activeModel?.modelId,
  )

  return (
    <div className="px-3 pb-3 pt-2">
        <div className="relative rounded-2xl border border-border bg-card">
          {/* Slash Command Menu */}
          {slash.isOpen && (
            <SlashCommandMenu
              commands={slash.filteredCommands}
              selectedIndex={slash.selectedIndex}
              onSelect={slash.executeCommand}
            />
          )}
          {/* Attached files preview */}
          {attachedFiles.length > 0 && (
            <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
              {attachedFiles.map((file, index) => (
                <div
                  key={`${file.name}-${index}`}
                  className="group relative flex items-center gap-1.5 bg-secondary rounded-lg px-2 py-1 text-xs text-foreground/80 border border-border/50 animate-in fade-in-0 slide-in-from-bottom-1 duration-150"
                  style={{ animationDelay: `${index * 50}ms`, animationFillMode: "both" }}
                >
                  {file.type.startsWith("image/") ? (
                    <img
                      src={URL.createObjectURL(file)}
                      alt={file.name}
                      className="h-8 w-8 rounded object-cover cursor-zoom-in"
                      onClick={(e) => {
                        e.stopPropagation()
                        openLightbox(URL.createObjectURL(file), file.name)
                      }}
                    />
                  ) : (
                    <Paperclip className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                  )}
                  <span className="max-w-[120px] truncate">{file.name}</span>
                  <button
                    className="ml-0.5 text-muted-foreground hover:text-foreground transition-colors"
                    onClick={() => onRemoveFile(index)}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Pending message indicator */}
          {loading && pendingMessage && (
            <div className="flex items-center gap-2 px-4 pt-2 pb-0 animate-in fade-in-0 slide-in-from-top-1 duration-200">
              <div className="flex items-center gap-1.5 bg-amber-500/10 text-amber-600 dark:text-amber-400 rounded-lg px-2.5 py-1 text-xs">
                <div className="h-1.5 w-1.5 rounded-full bg-amber-500 animate-pulse" />
                <span className="truncate max-w-[300px]">{pendingMessage}</span>
                <button
                  className="hover:text-foreground transition-colors"
                  onClick={onCancelPending}
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            </div>
          )}

          {/* Plan Mode Banner */}
          {planState === "planning" && (
            <div className="flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200">
              <ClipboardList className="h-3.5 w-3.5 shrink-0" />
              <span className="flex-1">{t("planMode.restricted")}</span>
              <button onClick={onExitPlanMode} className="hover:text-blue-800 dark:hover:text-blue-200 transition-colors">
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}

          {/* Textarea */}
          <Textarea
            ref={textareaRef}
            placeholder={
              planState === "planning"
                ? t("planMode.placeholder")
                : loading && pendingMessage
                  ? t("chat.pendingQueued")
                  : t("chat.askAnything")
            }
            value={input}
            onChange={(e) => onInputChange(e.target.value)}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            rows={2}
            className="border-0 shadow-none bg-transparent px-4 pt-3 pb-1 text-sm text-foreground placeholder:text-muted-foreground focus-visible:ring-0 resize-none min-h-[52px] max-h-[200px]"
          />

          {/* Toolbar */}
          <div className="flex items-center gap-1 px-2 pb-2 flex-wrap">
            {/* Attach buttons */}
            <IconTip label={t("chat.attachImage")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
                onClick={() => imageInputRef.current?.click()}
              >
                <ImagePlus className="h-4 w-4" />
              </Button>
            </IconTip>
            <input
              ref={imageInputRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={handleFileSelect}
            />
            <IconTip label={t("chat.attachFile")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
                onClick={() => fileInputRef.current?.click()}
              >
                <Paperclip className="h-4 w-4" />
              </Button>
            </IconTip>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              className="hidden"
              onChange={handleFileSelect}
            />

            {/* Slash Command Button */}
            <IconTip label={t("slashCommands.buttonTip")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
                onClick={() => slash.setOpen(!slash.isOpen)}
              >
                <Slash className="h-3.5 w-3.5" />
              </Button>
            </IconTip>

            {/* Model Selector */}
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

                {/* Cascading menu */}
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

            {/* Think Mode Toggle */}
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

            {/* Plan Mode Toggle */}
            <IconTip label={planState === "off" ? t("planMode.enter") : t("planMode.indicator")}>
              <button
                onClick={() => planState === "off" ? onEnterPlanMode?.() : onTogglePlanPanel?.()}
                className={cn(
                  "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
                  planState === "planning"
                    ? "text-blue-600 bg-blue-500/10"
                    : planState === "executing"
                    ? "text-green-600 bg-green-500/10"
                    : "text-muted-foreground hover:text-foreground"
                )}
              >
                <ClipboardList className="h-3.5 w-3.5 shrink-0" />
                {planState !== "off" && (
                  <span>{planState === "planning" ? t("planMode.indicator") : `${planProgress}%`}</span>
                )}
              </button>
            </IconTip>

            {/* Tool Permission Mode */}
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

            <div className="flex-1" />

            {/* Stop Button (always visible during loading) */}
            {loading && (
              <div className="animate-in fade-in-0 zoom-in-90 duration-150">
                <IconTip label={t("chat.stopReply")}>
                  <Button
                    size="icon"
                    variant="destructive"
                    className="h-8 w-8 rounded-full shrink-0"
                    onClick={onStop}
                  >
                    <Square className="h-4 w-4 fill-white stroke-white" />
                  </Button>
                </IconTip>
              </div>
            )}

            {/* Send Button */}
            <IconTip label={loading && input.trim() ? t("chat.queueMessage") : t("chat.send")}>
              <Button
                size="icon"
                className="h-8 w-8 rounded-full shrink-0"
                onClick={onSend}
                disabled={!input.trim() || (loading && !!pendingMessage)}
              >
                <Send className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
        </div>
      </div>
  )
}
