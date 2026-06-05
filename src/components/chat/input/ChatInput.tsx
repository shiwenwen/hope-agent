import { Fragment, useRef, useEffect, useCallback, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Textarea } from "@/components/ui/textarea"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
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
  FolderPlus,
  Quote,
} from "lucide-react"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import type {
  AvailableModel,
  ActiveModel,
  ChatTurnStatus,
  SessionMode,
  PendingFileQuote,
} from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
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
import PermissionModeSwitcher from "./PermissionModeSwitcher"
import TemperatureSlider from "./TemperatureSlider"
import AwarenessToggle from "./AwarenessToggle"
import WorkingDirectoryButton from "./WorkingDirectoryButton"
import { VoiceRecordButton } from "./VoiceRecordButton"
import { useVoiceInput } from "./useVoiceInput"
import { RecordingBar } from "./RecordingBar"
import { getNextPermissionMode } from "./permissionModes"
import WorkspaceStatusBar from "@/components/chat/workspace/WorkspaceStatusBar"
import { resolveWorkspaceTaskExecutionState } from "@/components/chat/workspace/taskExecutionState"
import {
  shouldShowTaskProgressPanel,
  type TaskProgressSnapshot,
} from "@/components/chat/tasks/taskProgress"
import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_BREAKPOINT_PX,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX,
  getChatInputOverflowActionIds,
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
  pendingQuotes?: PendingFileQuote[]
  onRemoveQuote?: (index: number) => void
  /** Click a staged quote chip to reveal that file in the file browser. */
  onJumpToQuote?: (q: PendingFileQuote) => void
  pendingMessage?: string | null
  onCancelPending?: () => void
  onDiscardPending?: () => void
  onStop?: () => void
  // Slash command support
  currentSessionId?: string | null
  currentAgentId?: string
  onCommandAction?: (result: CommandResult) => void
  // Tool permission mode
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode) => void
  // Temperature
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (temp: number | null) => void
  // Incognito
  incognitoEnabled?: boolean
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
  planState?: "off" | "planning" | "review" | "executing" | "completed"
  onEnterPlanMode?: () => void
  onExitPlanMode?: () => void
  onTogglePlanPanel?: () => void
  // Session-scoped Todo progress
  taskProgressSnapshot?: TaskProgressSnapshot | null
  executionState?: ChatTurnStatus | null
  /** 打开右侧工作台面板（状态条点击）。 */
  onOpenWorkspace?: () => void
  /** Larger centered presentation for a brand-new empty conversation. */
  hero?: boolean
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
  pendingQuotes,
  onRemoveQuote,
  onJumpToQuote,
  pendingMessage,
  onCancelPending,
  onDiscardPending,
  onStop,
  currentSessionId,
  currentAgentId = DEFAULT_AGENT_ID,
  onCommandAction,
  permissionMode,
  onPermissionModeChange,
  sessionTemperature,
  onSessionTemperatureChange,
  incognitoEnabled = false,
  workingDir,
  workingDirInherited = false,
  workingDirSaving = false,
  onWorkingDirChange,
  planState = "off",
  onEnterPlanMode,
  onExitPlanMode,
  onTogglePlanPanel,
  taskProgressSnapshot,
  executionState,
  onOpenWorkspace,
  hero = false,
}: ChatInputProps) {
  const { t } = useTranslation()
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const inputShellRef = useRef<HTMLDivElement>(null)
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)
  const [toolbarCompact, setToolbarCompact] = useState(false)
  const [toolbarStacked, setToolbarStacked] = useState(false)

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
  }
  const slash = useSlashCommands(input, onInputChange, slashActions)
  const voice = useVoiceInput()
  // Read the latest `input` when transcription resolves — the user can keep
  // typing during the STT round-trip, and capturing `input` in the closure
  // would overwrite anything typed in the meantime.
  const inputRef = useRef(input)
  useEffect(() => {
    inputRef.current = input
  }, [input])

  /**
   * Caret anchor captured at `voice.start()` time. While recording, the
   * transcript (streaming partial OR batch final) is spliced INTO the
   * textarea at this position rather than appended to the end. Cleared
   * after stop / cancel.
   */
  const voiceAnchorRef = useRef<{ prefix: string; suffix: string } | null>(null)

  const startVoice = useCallback(async () => {
    const ta = textareaRef.current
    const current = inputRef.current
    const selStart = ta?.selectionStart ?? current.length
    const selEnd = ta?.selectionEnd ?? current.length
    voiceAnchorRef.current = {
      prefix: current.slice(0, selStart),
      suffix: current.slice(selEnd),
    }
    await voice.start()
  }, [voice])

  const handleVoiceStop = useCallback(async () => {
    const text = await voice.stopAndTranscribe()
    const anchor = voiceAnchorRef.current
    voiceAnchorRef.current = null
    if (!text) {
      // Failed / empty transcript — restore the surrounding text in case
      // streaming partials had already written something.
      if (anchor) onInputChange(anchor.prefix + anchor.suffix)
      return
    }
    const prefix = anchor?.prefix ?? inputRef.current
    const suffix = anchor?.suffix ?? ""
    onInputChange(prefix + text + suffix)
    // Restore caret to the end of the inserted transcript on the next
    // tick (after React commits the new value to the textarea).
    const caret = prefix.length + text.length
    requestAnimationFrame(() => {
      const ta = textareaRef.current
      if (!ta) return
      ta.focus()
      ta.setSelectionRange(caret, caret)
    })
  }, [voice, onInputChange])

  const handleVoiceCancel = useCallback(() => {
    const anchor = voiceAnchorRef.current
    voiceAnchorRef.current = null
    voice.cancel()
    // Strip any streaming partial that already landed in the textarea.
    if (anchor) onInputChange(anchor.prefix + anchor.suffix)
  }, [voice, onInputChange])

  // Streaming partials arrive at 10-30 Hz from WS providers. Writing
  // them straight into `input` triggers urlPreview / mention / slash
  // re-derivation on every frame, which thrashes the input area.
  // Coalesce to one commit per animation frame: hold the latest partial
  // in a ref, schedule a single rAF, splice once.
  const pendingPartialRef = useRef<string | null>(null)
  const partialRafRef = useRef<number | null>(null)
  useEffect(() => {
    if (voice.state !== "recording") return
    const anchor = voiceAnchorRef.current
    if (!anchor) return
    const partial = voice.partialText
    if (!partial) return
    pendingPartialRef.current = partial
    if (partialRafRef.current !== null) return
    partialRafRef.current = requestAnimationFrame(() => {
      partialRafRef.current = null
      const p = pendingPartialRef.current
      pendingPartialRef.current = null
      if (p == null) return
      const a = voiceAnchorRef.current
      if (!a) return
      onInputChange(a.prefix + p + a.suffix)
    })
  }, [voice.partialText, voice.state, onInputChange])
  useEffect(
    () => () => {
      if (partialRafRef.current !== null) cancelAnimationFrame(partialRafRef.current)
      partialRafRef.current = null
      pendingPartialRef.current = null
    },
    [],
  )

  // Press-to-talk: hold Ctrl+Shift+H anywhere on the page to dictate.
  // Mirrors the inline tooltip on `VoiceRecordButton`. Click-toggle still
  // works via the button itself; this is the keyboard-only path.
  const pttActiveRef = useRef(false)
  const voiceRef = useRef(voice)
  voiceRef.current = voice
  const handleVoiceStopRef = useRef(handleVoiceStop)
  handleVoiceStopRef.current = handleVoiceStop
  const handleVoiceCancelRef = useRef(handleVoiceCancel)
  handleVoiceCancelRef.current = handleVoiceCancel
  const startVoiceRef = useRef(startVoice)
  startVoiceRef.current = startVoice
  useEffect(() => {
    const isPttCombo = (e: KeyboardEvent) =>
      e.code === "KeyH" && e.shiftKey && e.ctrlKey && !e.altKey && !e.metaKey

    const onKeyDown = (e: KeyboardEvent) => {
      if (!isPttCombo(e)) return
      // OS sends repeats while the key is held — only the first edge matters.
      if (e.repeat) return
      e.preventDefault()
      if (pttActiveRef.current) return
      const s = voiceRef.current.state
      if (s !== "idle" && s !== "ready" && s !== "stopped" && s !== "error") return
      pttActiveRef.current = true
      logger.info("voice", "ChatInput::ptt", "start recording (ptt down)")
      void startVoiceRef.current()
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (!isPttCombo(e)) return
      if (!pttActiveRef.current) return
      pttActiveRef.current = false
      e.preventDefault()
      if (voiceRef.current.state === "recording") {
        logger.info("voice", "ChatInput::ptt", "stop recording (ptt up)")
        void handleVoiceStopRef.current()
      }
    }
    // Switching apps or alt-tabbing can swallow the keyup — fall back to
    // cancel so half-captured audio doesn't ride into the next session.
    const onBlur = () => {
      if (!pttActiveRef.current) return
      pttActiveRef.current = false
      if (voiceRef.current.state === "recording") {
        logger.warn("voice", "ChatInput::ptt", "blur during ptt: cancel recording")
        handleVoiceCancelRef.current()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("keyup", onKeyUp)
    window.addEventListener("blur", onBlur)
    return () => {
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("keyup", onKeyUp)
      window.removeEventListener("blur", onBlur)
    }
  }, [])

  // File mention `@` popper — only meaningful when a working dir is set.
  const mention = useFileMention(input, onInputChange, textareaRef, workingDir ?? null)
  const [mirrorScrollTop, setMirrorScrollTop] = useState(0)

  // URL preview
  const { previews: urlPreviews, dismissedUrls, dismiss: dismissUrl } = useUrlPreview(input)
  const hasSendableContent =
    input.trim().length > 0 || attachedFiles.length > 0 || (pendingQuotes?.length ?? 0) > 0

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

  // The chat column can shrink when a right-side panel opens while the viewport
  // stays wide, so the overflow affordance has to follow the input container
  // width instead of a viewport media query.
  useEffect(() => {
    const el = inputShellRef.current
    if (!el || typeof window === "undefined") return

    const update = (width = el.getBoundingClientRect().width) => {
      setToolbarCompact(width <= CHAT_INPUT_OVERFLOW_BREAKPOINT_PX)
      setToolbarStacked(width <= CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX)
    }

    update()

    if (typeof ResizeObserver === "undefined") {
      const handleResize = () => update()
      window.addEventListener("resize", handleResize)
      return () => window.removeEventListener("resize", handleResize)
    }

    const observer = new ResizeObserver((entries) => {
      update(entries[0]?.contentRect.width)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (showOverflowMenu && !toolbarCompact) setShowOverflowMenu(false)
  }, [showOverflowMenu, toolbarCompact])

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
    // Slash menu first (owns header `/...` slot), then mention popper, then
    // local chat shortcuts.
    if (slash.handleKeyDown(e)) return
    if (mention.handleKeyDown(e)) return
    if (
      e.key === "Tab" &&
      e.shiftKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.metaKey
    ) {
      e.preventDefault()
      onPermissionModeChange(getNextPermissionMode(permissionMode))
      return
    }
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
      case "completed":
        return t("planMode.enter")
      case "planning":
        return t("planMode.indicator")
      case "review":
        return t("planMode.review.badge")
      case "executing":
        return t("planMode.executing")
    }
  })()

  const overflowMenuItemClass =
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-50"

  const toggleSlashCommandMenu = () => {
    slash.setOpen(!slash.isOpen)
  }

  const taskExecutionState = resolveWorkspaceTaskExecutionState(executionState, loading)
  // 状态条是否会渲染（WorkspaceStatusBar 内部同款判断）——决定其下方 Plan
  // Banner 是否需要补顶部圆角。
  const hasVisibleTaskBar = shouldShowTaskProgressPanel(taskProgressSnapshot)

  const renderInlineAddControls = () => (
    <>
      {onWorkingDirChange && (
        <WorkingDirectoryButton
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
          <Slash className="h-4 w-4" />
        </Button>
      </IconTip>
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
            workingDir={workingDir ?? null}
            inherited={workingDirInherited}
            saving={workingDirSaving}
            variant="menu"
            onPicked={() => setShowOverflowMenu(false)}
            onChange={onWorkingDirChange}
          />
        ) : (
          <button type="button" disabled className={overflowMenuItemClass}>
            <FolderPlus className="h-4 w-4 shrink-0 text-muted-foreground" />
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
            <Slash className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("slashCommands.buttonTip")}</span>
          </button>
        )
    }
  }

  const renderOverflowMenuItems = () => (
    <>
      {getChatInputOverflowActionIds().map((actionId) => (
        <Fragment key={actionId}>{renderOverflowMenuItem(actionId)}</Fragment>
      ))}
    </>
  )

  return (
    <div className={cn("min-w-0 px-3 pb-3 pt-2", hero && "px-0 pb-0 pt-0")}>
      <div
        ref={inputShellRef}
        className={cn(
          "relative min-w-0 overflow-hidden rounded-input-dock border border-border-soft bg-surface-floating shadow-input-dock",
          hero && "shadow-floating",
        )}
      >
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

        <WorkspaceStatusBar
          snapshot={taskProgressSnapshot}
          executionState={taskExecutionState}
          onOpen={onOpenWorkspace ?? (() => {})}
        />

        {/* Attached files preview (rendered above textarea) */}
        <AttachmentPreview attachedFiles={attachedFiles} onRemoveFile={onRemoveFile} />

        {/* Staged "quote to chat" references */}
        <AnimatedCollapse open={!!pendingQuotes?.length}>
          <div className="flex flex-wrap gap-1.5 px-3 pt-2">
            {pendingQuotes?.map((q, index) => {
              const lines =
                q.startLine === q.endLine ? `${q.startLine}` : `${q.startLine}-${q.endLine}`
              return (
                <span
                  key={`${q.path}:${lines}:${index}`}
                  className="inline-flex max-w-[260px] items-center gap-0.5 rounded-md border border-border/60 bg-secondary/40 py-0.5 pl-1 pr-1 text-xs text-foreground/80"
                >
                  <IconTip label={t("fileBrowser.jumpToFile", "Show in file browser")}>
                    <button
                      type="button"
                      onClick={() => onJumpToQuote?.(q)}
                      disabled={!onJumpToQuote}
                      className="inline-flex min-w-0 items-center gap-1 rounded px-1 py-0.5 transition-colors hover:bg-background/70 disabled:pointer-events-none"
                    >
                      <Quote className="h-3 w-3 shrink-0 text-muted-foreground" />
                      <span className="truncate">
                        {q.name}
                        <span className="ml-1 text-muted-foreground">L{lines}</span>
                      </span>
                    </button>
                  </IconTip>
                  {onRemoveQuote && (
                    <button
                      type="button"
                      onClick={() => onRemoveQuote(index)}
                      className="rounded p-0.5 text-muted-foreground hover:bg-background/70 hover:text-foreground"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </span>
              )
            })}
          </div>
        </AnimatedCollapse>

        {/* Pending message card */}
        <AnimatedCollapse open={loading && !!pendingMessage}>
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
                    className="min-w-[140px] bg-surface-floating/95 backdrop-blur-xl border border-border-soft rounded-floating shadow-floating p-1.5 z-50 animate-in fade-in-0 zoom-in-95 duration-150"
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
        </AnimatedCollapse>

        {/* Plan Mode Banner */}
        <AnimatedCollapse open={planState === "planning"}>
          <div
            className={`flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200${!hasVisibleTaskBar && attachedFiles.length === 0 && !(loading && pendingMessage) ? " rounded-t-2xl" : ""}`}
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
        </AnimatedCollapse>

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
            rows={hero ? 2 : 1}
            // Lock input while recording — the waveform bar replaces
            // direct typing, and the anchor-splice depends on the prefix
            // / suffix captured at start time remaining stable.
            readOnly={voice.state === "recording" || voice.state === "transcribing"}
            className={cn(
              "relative border-0 shadow-none bg-transparent px-4 pt-3 pb-1 text-sm leading-[1.5] text-foreground placeholder:text-muted-foreground focus-visible:ring-0 resize-none min-h-[42px] max-h-[40vh] overflow-y-auto break-words",
              hero && "min-h-[72px] pt-4 pb-2",
            )}
          />
        </div>

        {/* URL Previews */}
        <AnimatedCollapse open={urlPreviews.size > 0}>
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
        </AnimatedCollapse>

        {/* Toolbar — replaced by RecordingBar while voice capture / STT
            is in flight, since the normal toolbar buttons are
            unreachable during recording anyway. */}
        <AnimatedCollapse open={voice.state === "recording" || voice.state === "transcribing"}>
          <RecordingBar
            transcribing={voice.state === "transcribing"}
            durationMs={voice.durationMs}
            levels={voice.levels}
            onCancel={handleVoiceCancel}
            onStop={() => void handleVoiceStop()}
          />
        </AnimatedCollapse>
        <AnimatedCollapse
          open={voice.state !== "recording" && voice.state !== "transcribing"}
          overflow="visible-when-open"
        >
        <div
          className={cn(
            "flex gap-2 px-2 pb-2 animate-in fade-in-0 slide-in-from-bottom-1 duration-150",
            toolbarStacked ? "flex-col items-stretch" : "items-end justify-between",
          )}
        >
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <div className={toolbarCompact ? "hidden" : CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS}>
              {renderInlineAddControls()}
            </div>

            <div className={toolbarCompact ? "block" : CHAT_INPUT_OVERFLOW_MENU_CLASS}>
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
                    className="z-50 min-w-[180px] overflow-hidden rounded-floating border border-border-soft bg-surface-floating/95 p-1.5 text-popover-foreground shadow-floating backdrop-blur-xl animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
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
                  if (planState === "off" || planState === "completed") {
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
                        : "text-muted-foreground hover:text-foreground",
                )}
              >
                <ClipboardList className="h-4 w-4 shrink-0" />
                <span>{planToggleLabel}</span>
              </button>
            </IconTip>

            {/* Tool Permission Mode */}
            <PermissionModeSwitcher
              permissionMode={permissionMode}
              onPermissionModeChange={onPermissionModeChange}
            />
          </div>

          {/* Send & Stop — kept in its own column so toolbar wrapping never
              orphans the send button onto a half-empty row. */}
          <div
            className={cn(
              "flex items-center gap-1 shrink-0",
              toolbarStacked && "ml-auto",
            )}
          >
            <VoiceRecordButton
              state={voice.state}
              durationMs={voice.durationMs}
              audioLevel={voice.audioLevel}
              disabled={loading && !!pendingMessage}
              onStart={() => void startVoice()}
              onStop={() => void handleVoiceStop()}
              onCancel={handleVoiceCancel}
            />
            {voice.errorMessage && (
              <span
                className="text-xs text-destructive truncate max-w-[180px]"
                role="status"
                onAnimationEnd={voice.clearError}
              >
                {voice.errorMessage}
              </span>
            )}
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

            <IconTip label={loading && hasSendableContent ? t("chat.queueMessage") : t("chat.send")}>
              <Button
                size="icon"
                className="h-8 w-8 rounded-full shrink-0"
                onClick={onSend}
                disabled={!hasSendableContent || (loading && !!pendingMessage)}
                aria-label={loading && hasSendableContent ? t("chat.queueMessage") : t("chat.send")}
              >
                <Send className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
        </div>
        </AnimatedCollapse>
      </div>
    </div>
  )
}
