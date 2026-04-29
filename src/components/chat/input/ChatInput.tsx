import { Fragment, useRef, useEffect, useCallback, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import {
  Send,
  Square,
  Slash,
  ClipboardList,
  Pencil,
  Trash2,
  MoreHorizontal,
  BetweenHorizontalStart,
  X,
  Plus,
  Ghost,
  Loader2,
  FolderPlus,
} from "lucide-react"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import type { AvailableModel, ActiveModel, ToolPermissionMode } from "@/types/chat"
import { useSlashCommands, type SlashCommandActions } from "../slash-commands/useSlashCommands"
import { useUrlPreview } from "@/hooks/useUrlPreview"
import SlashCommandMenu from "../slash-commands/SlashCommandMenu"
import { useFileMention } from "../file-mention/useFileMention"
import FileMentionMenu from "../file-mention/FileMentionMenu"
import MentionMirrorOverlay from "../file-mention/MentionMirrorOverlay"
import UrlPreviewCard from "../UrlPreviewCard"
import type { CommandResult } from "../slash-commands/types"
import {
  AttachFileButton,
  AttachFilesMenuItem,
  AttachImageButton,
  AttachmentPreview,
} from "./AttachmentBar"
import ModelPicker from "./ModelPicker"
import ToolPermissionToggle from "./ToolPermissionToggle"
import TemperatureSlider from "./TemperatureSlider"
import AwarenessToggle from "./AwarenessToggle"
import IncognitoToggle, { type IncognitoDisabledReason } from "./IncognitoToggle"
import WorkingDirectoryButton from "./WorkingDirectoryButton"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import {
  shouldShowTaskProgressPanel,
  type TaskProgressSnapshot,
} from "@/components/chat/tasks/taskProgress"
import { Switch } from "@/components/ui/switch"
import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_BREAKPOINT_PX,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  getChatInputOverflowActionIds,
  shouldShowIncognitoPresetAction,
  type ChatInputOverflowActionId,
} from "./toolbarOverflow"

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
  /** True when `workingDir` is inherited from the parent project rather than
   *  set on the session itself; used to suppress the "clear" affordance in
   *  WorkingDirectoryButton (clearing a session value that is already null
   *  is a no-op the user can't observe). */
  workingDirInherited?: boolean
  workingDirSaving?: boolean
  onWorkingDirChange?: (workingDir: string | null) => void
  // Plan mode
  planState?: "off" | "planning" | "review" | "executing" | "paused" | "completed"
  planProgress?: number
  onEnterPlanMode?: () => void
  onExitPlanMode?: () => void
  onTogglePlanPanel?: () => void
  // Session-scoped Todo progress
  taskProgressSnapshot?: TaskProgressSnapshot | null
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
  workingDirInherited = false,
  workingDirSaving = false,
  onWorkingDirChange,
  planState = "off",
  planProgress = 0,
  onEnterPlanMode,
  onExitPlanMode,
  onTogglePlanPanel,
  taskProgressSnapshot,
}: ChatInputProps) {
  const { t } = useTranslation()
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
  }
  const slash = useSlashCommands(input, onInputChange, slashActions)

  // File mention `@` popper — only meaningful when a working dir is set.
  const mention = useFileMention(input, onInputChange, textareaRef, workingDir ?? null)
  const [mirrorScrollTop, setMirrorScrollTop] = useState(0)

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

  // The overflow `+` trigger is hidden via CSS once the viewport widens past
  // the breakpoint, but Radix keeps the open popper mounted in a Portal — with
  // no measurable trigger it falls back to (0,0). Listen for the breakpoint
  // crossing and force-close the menu when it goes wide.
  useEffect(() => {
    if (!showOverflowMenu) return
    if (typeof window === "undefined" || !window.matchMedia) return
    const mql = window.matchMedia(`(min-width: ${CHAT_INPUT_OVERFLOW_BREAKPOINT_PX + 1}px)`)
    const onChange = (e: MediaQueryListEvent) => {
      if (e.matches) setShowOverflowMenu(false)
    }
    mql.addEventListener("change", onChange)
    return () => mql.removeEventListener("change", onChange)
  }, [showOverflowMenu])

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
    // Slash menu first (owns header `/...` slot), then mention popper, then send.
    if (slash.handleKeyDown(e)) return
    if (mention.handleKeyDown(e)) return
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      onSend()
    }
  }

  const currentModelInfo = availableModels.find(
    (m) => m.providerId === activeModel?.providerId && m.modelId === activeModel?.modelId,
  )
  const planToggleLabel = t("planMode.toggleLabel")
  const planToggleTip = (() => {
    switch (planState) {
      case "off":
        return t("planMode.enter")
      case "planning":
        return t("planMode.indicator")
      case "review":
        return t("planMode.review.badge")
      case "executing":
        return `${t("planMode.executing")} ${planProgress}%`
      case "paused":
        return t("planMode.paused.badge")
      case "completed":
        return t("planMode.completed")
    }
  })()

  const overflowMenuItemClass =
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-50"
  const showIncognitoPreset = shouldShowIncognitoPresetAction(
    currentSessionId ?? null,
    !!onIncognitoChange,
  )

  const toggleSlashCommandMenu = () => {
    slash.setOpen(!slash.isOpen)
  }

  const toggleIncognito = (next = !incognitoEnabled) => {
    if (
      !onIncognitoChange ||
      !showIncognitoPreset ||
      incognitoSaving ||
      incognitoDisabledReason !== undefined
    ) {
      return
    }
    onIncognitoChange(next)
  }

  const visibleTaskProgress = shouldShowTaskProgressPanel(taskProgressSnapshot)
    ? taskProgressSnapshot
    : null

  const renderInlineAddControls = () => (
    <>
      {onWorkingDirChange && (
        <WorkingDirectoryButton
          sessionId={currentSessionId ?? null}
          workingDir={workingDir ?? null}
          inherited={workingDirInherited}
          saving={workingDirSaving}
          onChange={onWorkingDirChange}
        />
      )}
      <AttachImageButton onAttachFiles={onAttachFiles} />
      <AttachFileButton onAttachFiles={onAttachFiles} />
      <IconTip label={t("slashCommands.buttonTip")}>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
          onClick={toggleSlashCommandMenu}
        >
          <Slash className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      {showIncognitoPreset && onIncognitoChange && (
        <IncognitoToggle
          sessionId={currentSessionId ?? null}
          enabled={incognitoEnabled}
          saving={incognitoSaving}
          disabledReason={incognitoDisabledReason}
          onChange={onIncognitoChange}
        />
      )}
    </>
  )

  const renderOverflowMenuItem = (actionId: ChatInputOverflowActionId) => {
    switch (actionId) {
      case "attach-files":
        return (
          <AttachFilesMenuItem
            onAttachFiles={onAttachFiles}
            onPicked={() => setShowOverflowMenu(false)}
          />
        )
      case "working-dir":
        return onWorkingDirChange ? (
          <WorkingDirectoryButton
            sessionId={currentSessionId ?? null}
            workingDir={workingDir ?? null}
            inherited={workingDirInherited}
            saving={workingDirSaving}
            variant="menu"
            onPicked={() => setShowOverflowMenu(false)}
            onChange={onWorkingDirChange}
          />
        ) : (
          <button type="button" disabled className={overflowMenuItemClass}>
            <FolderPlus className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("chat.addWorkingDirectory")}</span>
          </button>
        )
      case "slash-command":
        return (
          <button
            type="button"
            className={overflowMenuItemClass}
            onClick={() => {
              setShowOverflowMenu(false)
              toggleSlashCommandMenu()
            }}
          >
            <Slash className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("slashCommands.buttonTip")}</span>
          </button>
        )
      case "incognito": {
        if (!onIncognitoChange || !showIncognitoPreset) return null
        const disabled = incognitoSaving || incognitoDisabledReason !== undefined
        return (
          <div
            role="button"
            tabIndex={disabled ? -1 : 0}
            aria-disabled={disabled}
            className={cn(
              overflowMenuItemClass,
              "justify-between",
              disabled && "pointer-events-none opacity-50",
            )}
            onClick={() => toggleIncognito()}
            onKeyDown={(e) => {
              if (disabled) return
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault()
                toggleIncognito()
              }
            }}
          >
            <span className="flex min-w-0 items-center gap-3">
              {incognitoSaving ? (
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
              ) : (
                <Ghost className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              )}
              <span className="truncate">{t("chat.incognito")}</span>
            </span>
            <Switch
              checked={incognitoEnabled}
              disabled={disabled}
              onClick={(e) => e.stopPropagation()}
              onCheckedChange={(checked) => toggleIncognito(checked)}
            />
          </div>
        )
      }
    }
  }

  const renderOverflowMenuItems = () => (
    <>
      {getChatInputOverflowActionIds(currentSessionId ?? null, !!onIncognitoChange).map(
        (actionId) => (
          <Fragment key={actionId}>{renderOverflowMenuItem(actionId)}</Fragment>
        ),
      )}
    </>
  )

  return (
    <div className="px-3 pb-3 pt-2">
      <div className="relative rounded-2xl border border-border bg-white dark:bg-card">
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

        {/* File Mention Menu (`@` popper) */}
        <FileMentionMenu
          isOpen={mention.isOpen && !slash.isOpen}
          entries={mention.entries}
          selectedIndex={mention.selectedIndex}
          mode={mention.mode}
          dirPath={mention.dirPath}
          workingDir={workingDir ?? null}
          loading={mention.loading}
          error={mention.error}
          truncated={mention.truncated}
          onSelect={mention.applyEntry}
          onHover={mention.setSelectedIndex}
        />

        {visibleTaskProgress && (
          <TaskProgressPanel snapshot={visibleTaskProgress} variant="embedded" />
        )}

        {/* Attached files preview (rendered above textarea) */}
        <AttachmentPreview attachedFiles={attachedFiles} onRemoveFile={onRemoveFile} />

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
          <div
            className={`flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200${!visibleTaskProgress && attachedFiles.length === 0 && !(loading && pendingMessage) ? " rounded-t-2xl" : ""}`}
          >
            <ClipboardList className="h-3.5 w-3.5 shrink-0" />
            <span className="flex-1">{t("planMode.restricted")}</span>
            <button
              onClick={onExitPlanMode}
              className="hover:text-blue-800 dark:hover:text-blue-200 transition-colors"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* Textarea + mention chip mirror — `@` mentions get a chip backdrop
            from the overlay; the textarea renders the actual characters on
            top so caret/selection stay native. */}
        <div className="relative">
          <MentionMirrorOverlay
            value={input}
            scrollTop={mirrorScrollTop}
            enabled={!!workingDir}
            onRemoveMention={mention.removeMention}
          />
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
            onSelect={() => mention.recheckTrigger()}
            onClick={() => mention.recheckTrigger()}
            onScroll={(e) => setMirrorScrollTop(e.currentTarget.scrollTop)}
            rows={1}
            className="relative border-0 shadow-none bg-transparent px-4 pt-3 pb-1 text-sm leading-[1.5] text-foreground placeholder:text-muted-foreground focus-visible:ring-0 resize-none min-h-[42px] max-h-[40vh] overflow-y-auto break-words"
          />
        </div>

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
        <div className="flex items-end justify-between gap-2 px-2 pb-2">
          <div className="flex items-center gap-1 flex-wrap min-w-0">
            <div className={CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS}>{renderInlineAddControls()}</div>

            <div className={CHAT_INPUT_OVERFLOW_MENU_CLASS}>
              <DropdownMenu.Root open={showOverflowMenu} onOpenChange={setShowOverflowMenu}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenu.Trigger asChild>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={t("chat.moreActions")}
                        className="h-8 w-8 rounded-lg bg-transparent text-muted-foreground hover:bg-transparent hover:text-foreground focus-visible:ring-0 data-[state=open]:bg-transparent"
                      >
                        <Plus className="h-4 w-4" />
                      </Button>
                    </DropdownMenu.Trigger>
                  </TooltipTrigger>
                  <TooltipContent>{t("chat.moreActions")}</TooltipContent>
                </Tooltip>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content
                    className="z-50 min-w-[180px] overflow-hidden rounded-xl border border-border/60 bg-white p-1.5 text-popover-foreground shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150 dark:bg-popover/95"
                    side="top"
                    align="start"
                    sideOffset={8}
                  >
                    <div className="flex flex-col gap-0.5">{renderOverflowMenuItems()}</div>
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            </div>

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

            <AwarenessToggle sessionId={currentSessionId ?? null} disabled={incognitoEnabled} />

            {/* Plan Mode Toggle */}
            <IconTip label={planToggleTip}>
              <button
                aria-label={planToggleTip}
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
                            : "text-muted-foreground hover:text-foreground",
                )}
              >
                <ClipboardList className="h-3.5 w-3.5 shrink-0" />
                <span>{planToggleLabel}</span>
              </button>
            </IconTip>

            {/* Tool Permission Mode */}
            <ToolPermissionToggle
              toolPermissionMode={toolPermissionMode}
              onToolPermissionChange={onToolPermissionChange}
            />
          </div>

          {/* Send & Stop — kept in its own column so toolbar wrapping never
              orphans the send button onto a half-empty row. */}
          <div className="flex items-center gap-1 shrink-0">
            {loading && (
              <div className="animate-in fade-in-0 zoom-in-90 duration-150">
                <IconTip label={t("chat.stopReply")}>
                  <Button
                    size="icon"
                    variant="destructive"
                    className="h-8 w-8 rounded-full shrink-0"
                    onClick={onStop}
                    aria-label={t("chat.stopReply")}
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
                aria-label={loading && input.trim() ? t("chat.queueMessage") : t("chat.send")}
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
