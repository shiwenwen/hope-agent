import { useRef, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { Send, Square, Slash, ClipboardList, Pencil, Trash2, MoreHorizontal, BetweenHorizontalStart, X } from "lucide-react"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import type { AvailableModel, ActiveModel, ToolPermissionMode } from "@/types/chat"
import { useSlashCommands, type SlashCommandActions } from "../slash-commands/useSlashCommands"
import { useUrlPreview } from "@/hooks/useUrlPreview"
import SlashCommandMenu from "../slash-commands/SlashCommandMenu"
import UrlPreviewCard from "../UrlPreviewCard"
import type { CommandResult } from "../slash-commands/types"
import AttachmentButtons, { AttachmentPreview } from "./AttachmentBar"
import ModelPicker from "./ModelPicker"
import ToolPermissionToggle from "./ToolPermissionToggle"
import TemperatureSlider from "./TemperatureSlider"
import AwarenessToggle from "./AwarenessToggle"
import IncognitoToggle, { type IncognitoDisabledReason } from "./IncognitoToggle"
import WorkingDirectoryButton from "./WorkingDirectoryButton"

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
  onDiscardPending?: () => void
  onStop?: () => void
  // Slash command support
  currentSessionId?: string | null
  currentAgentId?: string
  onCommandAction?: (result: CommandResult) => void
  // Tool permission mode
  toolPermissionMode: ToolPermissionMode
  onToolPermissionChange: (mode: ToolPermissionMode) => void
  // Temperature
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (temp: number | null) => void
  // Incognito
  incognitoEnabled?: boolean
  incognitoSaving?: boolean
  incognitoDisabledReason?: IncognitoDisabledReason
  onIncognitoChange?: (enabled: boolean) => void
  // Working directory
  workingDir?: string | null
  workingDirSaving?: boolean
  onWorkingDirChange?: (workingDir: string | null) => void
  // Plan mode
  planState?: "off" | "planning" | "review" | "executing" | "paused" | "completed"
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
  onDiscardPending,
  onStop,
  currentSessionId,
  currentAgentId = "default",
  onCommandAction,
  toolPermissionMode,
  onToolPermissionChange,
  sessionTemperature,
  onSessionTemperatureChange,
  incognitoEnabled = false,
  incognitoSaving = false,
  incognitoDisabledReason,
  onIncognitoChange,
  workingDir,
  workingDirSaving = false,
  onWorkingDirChange,
  planState = "off",
  planProgress = 0,
  onEnterPlanMode,
  onExitPlanMode,
  onTogglePlanPanel,
}: ChatInputProps) {
  const { t } = useTranslation()
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
  }
  const slash = useSlashCommands(input, onInputChange, slashActions)

  // URL preview
  const { previews: urlPreviews, dismissedUrls, dismiss: dismissUrl } = useUrlPreview(input)

  // Auto-resize textarea based on content
  const adjustTextareaHeight = useCallback(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    textarea.style.height = "auto"
    textarea.style.height = `${textarea.scrollHeight}px`
  }, [])

  useEffect(() => {
    adjustTextareaHeight()
  }, [input, adjustTextareaHeight])

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
              commands={slash.expandedCmd ? [] : slash.filteredCommands}
              selectedIndex={slash.selectedIndex}
              onSelect={slash.executeCommand}
              expandedCmd={slash.expandedCmd}
              filteredOptions={slash.filteredOptions}
              selectedOptionIndex={slash.selectedOptionIndex}
              onSelectOption={slash.executeOption}
            />
          )}

          {/* Attached files preview (rendered above textarea) */}
          <AttachmentPreview
            attachedFiles={attachedFiles}
            onRemoveFile={onRemoveFile}
          />

          {/* Pending message card */}
          {loading && pendingMessage && (
            <div className="px-3 pt-2.5 pb-0 animate-in fade-in-0 slide-in-from-top-1 duration-200">
              <div className="flex items-center gap-2 bg-amber-500/8 border border-amber-500/20 rounded-xl px-3 py-2">
                <BetweenHorizontalStart className="h-4 w-4 text-amber-500 shrink-0" />
                <span className="flex-1 text-sm text-foreground/90 truncate">{pendingMessage}</span>
                <IconTip label={t("chat.pendingDelete")}>
                  <button
                    className="p-1 rounded-md text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors"
                    onClick={onDiscardPending}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </IconTip>
                <DropdownMenu.Root>
                  <DropdownMenu.Trigger asChild>
                    <button className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors">
                      <MoreHorizontal className="h-3.5 w-3.5" />
                    </button>
                  </DropdownMenu.Trigger>
                  <DropdownMenu.Portal>
                    <DropdownMenu.Content
                      className="min-w-[140px] bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] p-1.5 z-50 animate-in fade-in-0 zoom-in-95 duration-150"
                      sideOffset={6}
                      align="end"
                    >
                      <DropdownMenu.Item
                        className="flex items-center gap-2 px-2.5 py-1.5 text-[13px] text-foreground/80 rounded-md cursor-pointer transition-colors hover:bg-secondary/60 hover:text-foreground outline-none"
                        onSelect={onCancelPending}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                        {t("chat.pendingEdit")}
                      </DropdownMenu.Item>
                      <DropdownMenu.Item
                        className="flex items-center gap-2 px-2.5 py-1.5 text-[13px] text-foreground/80 rounded-md cursor-pointer transition-colors hover:bg-secondary/60 hover:text-foreground outline-none"
                        onSelect={onDiscardPending}
                      >
                        <X className="h-3.5 w-3.5" />
                        {t("chat.pendingDiscard")}
                      </DropdownMenu.Item>
                    </DropdownMenu.Content>
                  </DropdownMenu.Portal>
                </DropdownMenu.Root>
              </div>
            </div>
          )}

          {/* Plan Mode Banner */}
          {planState === "planning" && (
            <div className={`flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200${attachedFiles.length === 0 && !(loading && pendingMessage) ? " rounded-t-2xl" : ""}`}>
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
            rows={1}
            className="border-0 shadow-none bg-transparent px-4 pt-3 pb-1 text-sm text-foreground placeholder:text-muted-foreground focus-visible:ring-0 resize-none min-h-[42px] max-h-[40vh] overflow-y-auto"
          />

          {/* URL Previews */}
          {urlPreviews.size > 0 && (
            <div className="px-3 pb-1 flex flex-col gap-1.5 max-h-[200px] overflow-y-auto">
              {Array.from(urlPreviews.entries())
                .filter(([url]) => !dismissedUrls.has(url))
                .map(([url, data]) => (
                  <UrlPreviewCard
                    key={url}
                    data={data}
                    dismissible
                    onDismiss={() => dismissUrl(url)}
                  />
                ))}
            </div>
          )}

          {/* Toolbar */}
          <div className="flex items-center gap-1 px-2 pb-2 flex-wrap">
            {/* Attach buttons */}
            <AttachmentButtons onAttachFiles={onAttachFiles} />

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

            {/* Model Selector + Think Mode */}
            <ModelPicker
              availableModels={availableModels}
              activeModel={activeModel}
              reasoningEffort={reasoningEffort}
              onModelChange={onModelChange}
              onEffortChange={onEffortChange}
              currentModelInfo={currentModelInfo}
            />

            {/* Temperature Control */}
            <TemperatureSlider
              sessionTemperature={sessionTemperature}
              onSessionTemperatureChange={onSessionTemperatureChange}
            />

            {onIncognitoChange && (
              <IncognitoToggle
                sessionId={currentSessionId ?? null}
                enabled={incognitoEnabled}
                saving={incognitoSaving}
                disabledReason={incognitoDisabledReason}
                onChange={onIncognitoChange}
              />
            )}

            {onWorkingDirChange && (
              <WorkingDirectoryButton
                sessionId={currentSessionId ?? null}
                workingDir={workingDir ?? null}
                saving={workingDirSaving}
                onChange={onWorkingDirChange}
              />
            )}

            <AwarenessToggle
              sessionId={currentSessionId ?? null}
              disabled={incognitoEnabled}
            />

            {/* Plan Mode Toggle */}
            <IconTip label={planState === "off" ? t("planMode.enter") : t("planMode.indicator")}>
              <button
                onClick={() => {
                  if (planState === "off") {
                    onEnterPlanMode?.()
                  } else if (planState === "planning") {
                    onExitPlanMode?.()
                  } else {
                    onTogglePlanPanel?.()
                  }
                }}
                className={cn(
                  "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
                  planState === "planning"
                    ? "text-blue-600 bg-blue-500/10"
                    : planState === "review"
                    ? "text-purple-600 bg-purple-500/10"
                    : planState === "executing"
                    ? "text-green-600 bg-green-500/10"
                    : planState === "paused"
                    ? "text-yellow-600 bg-yellow-500/10"
                    : planState === "completed"
                    ? "text-green-600 bg-green-500/10"
                    : "text-muted-foreground hover:text-foreground"
                )}
              >
                <ClipboardList className="h-3.5 w-3.5 shrink-0" />
                {planState !== "off" && (
                  <span>
                    {planState === "planning" ? t("planMode.indicator")
                      : planState === "review" ? t("planMode.review.badge")
                      : planState === "paused" ? t("planMode.paused.badge")
                      : planState === "completed" ? t("planMode.completed")
                      : `${planProgress}%`}
                  </span>
                )}
              </button>
            </IconTip>

            {/* Tool Permission Mode */}
            <ToolPermissionToggle
              toolPermissionMode={toolPermissionMode}
              onToolPermissionChange={onToolPermissionChange}
            />

            {/* Send & Stop */}
            <div className="flex items-center gap-1 ml-auto">
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
      </div>
  )
}
