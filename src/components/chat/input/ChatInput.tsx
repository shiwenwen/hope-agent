import { Fragment, useRef, useEffect, useCallback, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { AnimatedCollapse, AnimatedPresenceBox } from "@/components/ui/animated-presence"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import {
  Send,
  Square,
  Slash,
  ClipboardList,
  Pencil,
  Trash2,
  BetweenHorizontalStart,
  ChevronDown,
  ChevronUp,
  X,
  Plus,
  FolderPlus,
  Quote,
  Undo2,
} from "lucide-react"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import type {
  AvailableModel,
  ActiveModel,
  ChatTurnStatus,
  SandboxMode,
  SessionMode,
  PendingFileQuote,
  PendingSendPreview,
  AgentSummaryForSidebar,
} from "@/types/chat"
import type { KbDraftAttachment } from "@/types/knowledge"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { useSlashCommands, type SlashCommandActions } from "../slash-commands/useSlashCommands"
import { useUrlPreview } from "@/hooks/useUrlPreview"
import SlashCommandMenu from "../slash-commands/SlashCommandMenu"
import { useFileMention } from "../file-mention/useFileMention"
import FileMentionMenu from "../file-mention/FileMentionMenu"
import { useNoteMention } from "../note-mention/useNoteMention"
import NoteMentionMenu from "../note-mention/NoteMentionMenu"
import QuickPromptMenu from "../quick-prompts/QuickPromptMenu"
import { useQuickPrompts } from "../quick-prompts/useQuickPrompts"
import UrlPreviewCard from "../UrlPreviewCard"
import type { CommandResult } from "../slash-commands/types"
import { AttachFilesButton, AttachFilesMenuItem, AttachmentPreview } from "./AttachmentBar"
import ModelPicker from "./ModelPicker"
import PermissionModeSwitcher, { type PermissionModeChangeOptions } from "./PermissionModeSwitcher"
import SandboxModeSwitcher from "./SandboxModeSwitcher"
import AwarenessToggle from "./AwarenessToggle"
import KnowledgePicker from "./KnowledgePicker"
import WorkingDirectoryButton from "./WorkingDirectoryButton"
import { VoiceRecordButton } from "./VoiceRecordButton"
import { useVoiceInput } from "./useVoiceInput"
import { RecordingBar } from "./RecordingBar"
import { getNextPermissionMode } from "./permissionModes"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import { resolveWorkspaceTaskExecutionState } from "@/components/chat/workspace/taskExecutionState"
import {
  shouldShowTaskProgressPanel,
  type TaskProgressSnapshot,
} from "@/components/chat/tasks/taskProgress"
import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_BREAKPOINT_PX,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_PERMISSION_COLLAPSE_BREAKPOINT_PX,
  CHAT_INPUT_SANDBOX_COLLAPSE_BREAKPOINT_PX,
  CHAT_INPUT_TIGHT_TOOLBAR_BREAKPOINT_PX,
  getChatInputOverflowActionIds,
  type ChatInputOverflowActionId,
} from "./toolbarOverflow"
import MentionComposerInput from "./MentionComposerInput"
import type { ComposerPasteEvent } from "./MentionComposerInput"
import type { ComposerInputHandle } from "./composerInputHandle"
import {
  createPastedTextAttachment,
  shouldCreatePastedTextAttachment,
} from "./pastedTextAttachment"
import type { ContextUsageInfo } from "../chatUtils"
import { contextUsageBarClass } from "../contextUsageColor"
import type { AgentConfig } from "@/components/settings/types"
import type { QuickPromptItem } from "@/types/quickPrompts"

interface ChatInputProps {
  input: string
  onInputChange: (value: string) => void
  inputHistory?: string[]
  quickPrompts?: QuickPromptItem[]
  onSend: () => void
  sendDisabled?: boolean
  loading: boolean
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  onModelChange: (key: string) => void
  onEffortChange: (effort: string) => void
  attachedFiles: File[]
  onAttachFiles: (files: File[]) => void
  onRemoveFile: (index: number) => void
  onUpdateFile: (index: number, file: File) => void
  pendingQuotes?: PendingFileQuote[]
  onRemoveQuote?: (index: number) => void
  /** Click a staged quote chip to reveal that file in the file browser. */
  onJumpToQuote?: (q: PendingFileQuote) => void
  pendingMessage?: string | null
  pendingSends?: PendingSendPreview[]
  onCancelPending?: () => void
  onDiscardPending?: () => void
  onEditPending?: (id: string) => void
  onDiscardPendingItem?: (id: string) => void
  onForceInsertPending?: (id: string) => void
  onCancelForceInsertPending?: (id: string) => void
  onStop?: () => void
  // Slash command support
  currentSessionId?: string | null
  currentAgentId?: string
  onCommandAction?: (result: CommandResult) => void
  // Tool permission mode
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode, options?: PermissionModeChangeOptions) => void
  // Sandbox execution mode
  sandboxMode: SandboxMode
  onSandboxModeChange: (mode: SandboxMode) => void
  // Temperature
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (temp: number | null) => void
  // Incognito
  incognitoEnabled?: boolean
  // Knowledge space attach (project context for project-scoped attaches)
  projectId?: string | null
  // Draft KB attaches staged before a session exists (composer draft mode)
  draftKbAttachments?: KbDraftAttachment[]
  onDraftKbAttachChange?: (next: KbDraftAttachment[]) => void
  /** Enable the `[[note]]` picker + the `@` menu's knowledge-notes section.
   *  Off by default so files-only surfaces (QuickChat) keep their behavior. */
  enableNoteMention?: boolean
  /** Enable the `@` menu's built-in **skills** section (`@skill:<name>` →
   *  office / browser / mac control). Off by default; the main chat opts in. */
  enableSkillMention?: boolean
  /** Enable the `@` menu's Agent delegation section (`@agent:<id>`). */
  enableAgentMention?: boolean
  agents?: AgentSummaryForSidebar[]
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
  /** True when the right-side workspace panel is expanded and showing task detail. */
  workspacePanelVisible?: boolean
  /** Larger centered presentation for a brand-new empty conversation. */
  hero?: boolean
  /** Context-window fullness, rendered as a thin bar fused into the dock's
   *  bottom border (green → amber → red). Null hides the bar. */
  contextUsage?: ContextUsageInfo | null
}

/**
 * Thin context-usage bar fused into the input dock's bottom border. The filled
 * width tracks how full the context window is; color ramps green → yellow → red
 * via the shared `contextUsageBarClass` (dependency-free leaf module, so the
 * input dock stays clear of chatUtils' heavier runtime chain). Sits in the
 * toolbar's `pb-2` padding zone (no buttons there), so its hover target never
 * steals clicks. Inset horizontally so the line starts after the dock's rounded
 * bottom corners instead of bleeding into the curved edge.
 */
function ContextUsageBottomBar({ usage }: { usage: ContextUsageInfo }) {
  const { t } = useTranslation()
  const fill = contextUsageBarClass(usage.pct)
  const width = Math.min(usage.pct, 100)
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className="absolute inset-x-4 bottom-0 z-10 flex h-2 items-end">
          <div className="h-[2px] w-full overflow-hidden rounded-full">
            <div
              className={cn("h-full rounded-full transition-[width] duration-500", fill)}
              style={{ width: `${width}%` }}
            />
          </div>
        </div>
      </TooltipTrigger>
      <TooltipContent side="top">
        {t("chat.statusContext")} · {usage.usedK}k/{usage.ctxK}k ({usage.pct}%)
      </TooltipContent>
    </Tooltip>
  )
}

export default function ChatInput({
  input,
  onInputChange,
  inputHistory = [],
  quickPrompts = [],
  onSend,
  sendDisabled = false,
  loading,
  availableModels,
  activeModel,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  attachedFiles,
  onAttachFiles,
  onRemoveFile,
  onUpdateFile,
  pendingQuotes,
  onRemoveQuote,
  onJumpToQuote,
  pendingMessage,
  pendingSends,
  onCancelPending,
  onDiscardPending,
  onEditPending,
  onDiscardPendingItem,
  onForceInsertPending,
  onCancelForceInsertPending,
  onStop,
  currentSessionId,
  currentAgentId = DEFAULT_AGENT_ID,
  onCommandAction,
  permissionMode,
  onPermissionModeChange,
  sandboxMode,
  onSandboxModeChange,
  sessionTemperature,
  onSessionTemperatureChange,
  incognitoEnabled = false,
  projectId,
  draftKbAttachments,
  onDraftKbAttachChange,
  enableNoteMention = false,
  enableSkillMention = false,
  enableAgentMention = false,
  agents = [],
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
  workspacePanelVisible = false,
  hero = false,
  contextUsage,
}: ChatInputProps) {
  const { t } = useTranslation()
  const inputHandleRef = useRef<ComposerInputHandle>(null)
  const inputShellRef = useRef<HTMLDivElement>(null)
  const toolbarRef = useRef<HTMLDivElement>(null)
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)
  const [toolbarCompact, setToolbarCompact] = useState(false)
  // Narrower tier than `toolbarCompact`: Knowledge + Plan stay inline until the
  // toolbar is genuinely cramped, then collapse into the "+" menu too.
  const [toolbarTight, setToolbarTight] = useState(false)
  // Progressively deeper tiers: sandbox collapses first, then permission mode.
  // The floor — "+" · model · send/stop — never collapses and never wraps.
  const [sandboxCollapsed, setSandboxCollapsed] = useState(false)
  const [permissionCollapsed, setPermissionCollapsed] = useState(false)
  const [toolbarMinHeight, setToolbarMinHeight] = useState<number | null>(null)
  const [pendingExpanded, setPendingExpanded] = useState(false)

  const handlePermissionModeChange = useCallback(
    (mode: SessionMode, options?: PermissionModeChangeOptions) => {
      onPermissionModeChange(mode, options)
      if (!options?.applyToAgentDefault) return

      void (async () => {
        try {
          const transport = getTransport()
          const config = await transport.call<AgentConfig>("get_agent_config", {
            id: currentAgentId,
          })
          await transport.call("save_agent_config_cmd", {
            id: currentAgentId,
            config: {
              ...config,
              capabilities: {
                ...config.capabilities,
                defaultSessionPermissionMode: mode,
              },
            },
          })
        } catch (e) {
          logger.error(
            "chat",
            "ChatInput::setAgentDefaultPermissionMode",
            "Failed to save agent default permission mode",
            e,
          )
          toast.error(t("chat.permissionMode.applyToAgentDefault.failed"))
        }
      })()
    },
    [currentAgentId, onPermissionModeChange, t],
  )

  const [historyIndex, setHistoryIndex] = useState<number | null>(null)
  const historyDraftRef = useRef("")

  const resetHistoryBrowsing = useCallback(() => {
    setHistoryIndex(null)
    historyDraftRef.current = ""
  }, [])

  const setComposerInput = useCallback(
    (value: string) => {
      resetHistoryBrowsing()
      onInputChange(value)
    },
    [onInputChange, resetHistoryBrowsing],
  )

  useEffect(() => {
    resetHistoryBrowsing()
  }, [currentSessionId, resetHistoryBrowsing])

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
  }
  const slash = useSlashCommands(input, setComposerInput, slashActions, inputHandleRef)
  const voice = useVoiceInput(currentSessionId)
  const normalToolbarOpen = voice.state !== "recording" && voice.state !== "transcribing"
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
   * composer at this position rather than appended to the end. Cleared
   * after stop / cancel.
   */
  const voiceAnchorRef = useRef<{ prefix: string; suffix: string } | null>(null)

  const startVoice = useCallback(async () => {
    const inputHandle = inputHandleRef.current
    const current = inputRef.current
    const selection = inputHandle?.getSelectionRange() ?? {
      start: current.length,
      end: current.length,
    }
    const selStart = selection.start
    const selEnd = selection.end
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
    // tick (after React commits the new value to the composer).
    const caret = prefix.length + text.length
    requestAnimationFrame(() => {
      const inputHandle = inputHandleRef.current
      if (!inputHandle) return
      inputHandle.focus()
      inputHandle.setSelectionRange(caret, caret)
    })
  }, [voice, onInputChange])

  const handleVoiceCancel = useCallback(() => {
    const anchor = voiceAnchorRef.current
    voiceAnchorRef.current = null
    voice.cancel()
    // Strip any streaming partial that already landed in the composer.
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

  // File mention `@` popper — files (working dir) + knowledge notes when enabled.
  const mention = useFileMention(
    input,
    setComposerInput,
    inputHandleRef,
    workingDir ?? null,
    enableNoteMention
      ? {
          sessionId: currentSessionId ?? null,
          projectId: projectId ?? null,
          draftKbAttachments: draftKbAttachments ?? [],
        }
      : undefined,
    enableSkillMention,
    enableAgentMention ? agents : [],
    currentAgentId,
  )
  // `[[note]]` picker — knowledge-space notes reachable from this chat.
  const noteMention = useNoteMention(
    input,
    setComposerInput,
    inputHandleRef,
    currentSessionId ?? null,
    projectId ?? null,
    draftKbAttachments ?? [],
    enableNoteMention,
  )
  // User-global quick prompts (`#` popper).
  const quickPrompt = useQuickPrompts(input, setComposerInput, inputHandleRef, quickPrompts)
  // URL preview
  const { previews: urlPreviews, dismissedUrls, dismiss: dismissUrl } = useUrlPreview(input)
  const hasSendableContent =
    input.trim().length > 0 || attachedFiles.length > 0 || (pendingQuotes?.length ?? 0) > 0

  // The chat column can shrink when a right-side panel opens while the viewport
  // stays wide, so the overflow affordance has to follow the input container
  // width instead of a viewport media query.
  useEffect(() => {
    const el = inputShellRef.current
    if (!el || typeof window === "undefined") return

    const update = (width = el.getBoundingClientRect().width) => {
      setToolbarCompact(width <= CHAT_INPUT_OVERFLOW_BREAKPOINT_PX)
      setToolbarTight(width <= CHAT_INPUT_TIGHT_TOOLBAR_BREAKPOINT_PX)
      setSandboxCollapsed(width <= CHAT_INPUT_SANDBOX_COLLAPSE_BREAKPOINT_PX)
      setPermissionCollapsed(width <= CHAT_INPUT_PERMISSION_COLLAPSE_BREAKPOINT_PX)
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

  useEffect(() => {
    setToolbarMinHeight(null)
  }, [toolbarCompact, sandboxCollapsed, permissionCollapsed])

  useEffect(() => {
    if (!normalToolbarOpen || typeof window === "undefined") {
      setToolbarMinHeight(null)
      return
    }

    const el = toolbarRef.current
    if (!el) return

    let raf: number | null = null
    const update = () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      raf = window.requestAnimationFrame(() => {
        raf = null
        const next = Math.ceil(el.scrollHeight || el.getBoundingClientRect().height)
        setToolbarMinHeight((prev) => {
          if (prev === null || next > prev) return next
          return prev
        })
      })
    }

    update()

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update)
      return () => {
        if (raf !== null) window.cancelAnimationFrame(raf)
        window.removeEventListener("resize", update)
      }
    }

    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      observer.disconnect()
    }
  }, [normalToolbarOpen, toolbarCompact, sandboxCollapsed, permissionCollapsed])

  const handlePaste = useCallback(
    (e: ComposerPasteEvent) => {
      const clipboardData = e.clipboardData
      if (!clipboardData) return
      const items = clipboardData.items
      const files: File[] = []
      if (items) {
        for (let i = 0; i < items.length; i++) {
          const item = items[i]
          if (item.kind === "file") {
            const file = item.getAsFile()
            if (file) files.push(file)
          }
        }
      }
      if (files.length > 0) {
        e.preventDefault()
        onAttachFiles(files)
        return
      }

      const pastedText = clipboardData.getData("text/plain")
      if (shouldCreatePastedTextAttachment(pastedText)) {
        e.preventDefault()
        onAttachFiles([createPastedTextAttachment(pastedText)])

        const selection = inputHandleRef.current?.getSelectionRange()
        if (selection && selection.start !== selection.end) {
          const current = inputRef.current
          const next = current.slice(0, selection.start) + current.slice(selection.end)
          setComposerInput(next)
          requestAnimationFrame(() => {
            const inputHandle = inputHandleRef.current
            if (!inputHandle) return
            inputHandle.focus()
            inputHandle.setSelectionRange(selection.start, selection.start)
          })
        }
      }
    },
    [onAttachFiles, setComposerInput],
  )

  const handleHistoryKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLElement>) => {
      if (
        (e.key !== "ArrowUp" && e.key !== "ArrowDown") ||
        e.shiftKey ||
        e.ctrlKey ||
        e.altKey ||
        e.metaKey ||
        inputHistory.length === 0
      ) {
        return false
      }

      const browsing = historyIndex !== null
      if (!browsing && input.length > 0) return false

      if (e.key === "ArrowDown" && !browsing) return false

      e.preventDefault()
      if (e.key === "ArrowUp") {
        const nextIndex =
          historyIndex == null ? 0 : Math.min(historyIndex + 1, inputHistory.length - 1)
        if (!browsing) historyDraftRef.current = input
        setHistoryIndex(nextIndex)
        onInputChange(inputHistory[nextIndex] ?? "")
        requestAnimationFrame(() => {
          const inputHandle = inputHandleRef.current
          const next = inputHistory[nextIndex] ?? ""
          if (!inputHandle) return
          inputHandle.focus()
          inputHandle.setSelectionRange(next.length, next.length)
        })
        return true
      }

      if (historyIndex == null) return false
      const nextIndex = historyIndex - 1
      if (nextIndex < 0) {
        const draft = historyDraftRef.current
        setHistoryIndex(null)
        historyDraftRef.current = ""
        onInputChange(draft)
        requestAnimationFrame(() => {
          const inputHandle = inputHandleRef.current
          if (!inputHandle) return
          inputHandle.focus()
          inputHandle.setSelectionRange(draft.length, draft.length)
        })
        return true
      }

      setHistoryIndex(nextIndex)
      onInputChange(inputHistory[nextIndex] ?? "")
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        const next = inputHistory[nextIndex] ?? ""
        if (!inputHandle) return
        inputHandle.focus()
        inputHandle.setSelectionRange(next.length, next.length)
      })
      return true
    },
    [historyIndex, input, inputHandleRef, inputHistory, onInputChange],
  )

  const sendUnavailable = sendDisabled || !hasSendableContent

  const handleSend = useCallback(() => {
    if (sendUnavailable) return
    resetHistoryBrowsing()
    onSend()
  }, [onSend, resetHistoryBrowsing, sendUnavailable])

  function handleKeyDown(e: React.KeyboardEvent<HTMLElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    // Slash menu first (owns header `/...` slot), then `[[note]]` picker, then
    // `@` file mention, then `#` quick prompts, then history/send. Each handler self-guards on its own open
    // state, so only the active popper consumes the key.
    if (slash.handleKeyDown(e)) return
    if (noteMention.handleKeyDown(e)) return
    if (mention.handleKeyDown(e)) return
    if (quickPrompt.handleKeyDown(e)) return
    if (handleHistoryKeyDown(e)) return
    if (e.key === "Tab" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault()
      onPermissionModeChange(getNextPermissionMode(permissionMode))
      return
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
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

  // Shared by the inline Plan toggle and its "+" overflow-menu counterpart.
  const handlePlanToggle = () => {
    if (planState === "off" || planState === "completed") {
      onEnterPlanMode?.()
    } else if (planState === "planning") {
      onExitPlanMode?.()
    } else {
      onTogglePlanPanel?.()
    }
  }

  const toggleSlashCommandMenu = () => {
    slash.setOpen(!slash.isOpen)
  }

  const taskExecutionState = resolveWorkspaceTaskExecutionState(executionState, loading)
  const visibleTaskProgressSnapshot = shouldShowTaskProgressPanel(taskProgressSnapshot)
    ? taskProgressSnapshot
    : null
  // 任务进度 UI 是否会渲染——决定其下方 Plan Banner 是否需要补顶部圆角。
  const hasVisibleTaskProgress = !!visibleTaskProgressSnapshot
  const pendingQueueItems: PendingSendPreview[] =
    pendingSends && pendingSends.length > 0
      ? pendingSends
      : pendingMessage
        ? [
            {
              id: "__legacy__",
              text: pendingMessage,
              mode: "queue",
              status: "queued",
              canForceInsert: false,
              attachmentCount: 0,
              quoteCount: 0,
            },
          ]
        : []
  const pendingVisibleItems = pendingExpanded ? pendingQueueItems : pendingQueueItems.slice(0, 2)
  const hasPendingQueue = loading && pendingQueueItems.length > 0

  const pendingStatusLabel = (item: PendingSendPreview) => {
    switch (item.status) {
      case "waiting_tool_boundary":
        return t("chat.pendingWaitingToolBoundary", "等待工具完成点")
      case "inserted":
        return t("chat.pendingInserted", "已插入")
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReply", "回复后发送")
      case "queued":
      default:
        return t("chat.pendingQueuedShort", "排队中")
    }
  }

  const pendingStatusTip = (item: PendingSendPreview) => {
    switch (item.status) {
      case "waiting_tool_boundary":
        return t(
          "chat.pendingWaitingToolBoundaryTip",
          "等待最近一次工具调用完成；如果本轮不再调用工具，将改为回复结束后发送。",
        )
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReplyTip", "未遇到工具完成点，将在当前回复结束后发送。")
      case "inserted":
        return t("chat.pendingInsertedTip", "已在本轮工具完成后插入给模型。")
      case "queued":
      default:
        return t("chat.pendingQueuedTip", "已加入待发送队列，将在当前回复结束后发送。")
    }
  }

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
      <AttachFilesButton onAttachFiles={onAttachFiles} />
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

  // Add-style actions (working dir / attach / slash) always live here once the
  // toolbar is compact. Knowledge + Plan only join the menu at the narrower
  // `toolbarTight` tier — above it they stay inline.
  const renderOverflowMenuItems = () => (
    <>
      {getChatInputOverflowActionIds().map((actionId) => (
        <Fragment key={actionId}>{renderOverflowMenuItem(actionId)}</Fragment>
      ))}
      {toolbarTight && (
        <>
          <KnowledgePicker
            variant="menu"
            sessionId={currentSessionId ?? null}
            projectId={projectId ?? null}
            disabled={incognitoEnabled}
            draftAttachments={draftKbAttachments}
            onDraftAttachChange={onDraftKbAttachChange}
          />
          <button
            type="button"
            aria-label={planToggleTip}
            className={cn(
              overflowMenuItemClass,
              planState === "planning" && "text-blue-600",
              planState === "review" && "text-purple-600",
              planState === "executing" && "text-green-600",
            )}
            onClick={() => {
              setShowOverflowMenu(false)
              handlePlanToggle()
            }}
          >
            <ClipboardList className="h-4 w-4 shrink-0" />
            <span className="truncate">{planToggleLabel}</span>
          </button>
        </>
      )}
      {sandboxCollapsed && (
        <SandboxModeSwitcher
          variant="menu"
          sandboxMode={sandboxMode}
          onSandboxModeChange={onSandboxModeChange}
        />
      )}
      {permissionCollapsed && (
        <PermissionModeSwitcher
          variant="menu"
          permissionMode={permissionMode}
          onPermissionModeChange={handlePermissionModeChange}
        />
      )}
    </>
  )

  return (
    <div className={cn("min-w-0 px-3 pb-3 pt-2", hero && "px-0 pb-0 pt-0")}>
      <div
        ref={inputShellRef}
        className={cn(
          "relative min-w-0 overflow-visible rounded-input-dock border border-border-soft bg-surface-floating shadow-input-dock",
          hero && "shadow-floating",
          incognitoEnabled &&
            [
              "[--color-surface-floating:hsl(0_0%_13%)]",
              "[--color-surface-subtle:hsl(0_0%_16%)]",
              "[--color-secondary:hsl(0_0%_17%)]",
              "[--color-foreground:hsl(0_0%_96%)]",
              "[--color-muted-foreground:hsl(0_0%_70%)]",
              "[--color-border:hsl(0_0%_22%)]",
              "[--color-border-soft:hsl(0_0%_22%)]",
              "shadow-[0_18px_52px_hsl(0_0%_4%/0.24)]",
            ],
        )}
      >
        {/* Slash Command Menu */}
        <SlashCommandMenu
          open={slash.isOpen}
          commands={slash.expandedCmd ? [] : slash.filteredCommands}
          selectedIndex={slash.selectedIndex}
          onSelect={slash.executeCommand}
          expandedCmd={slash.expandedCmd}
          filteredOptions={slash.filteredOptions}
          selectedOptionIndex={slash.selectedOptionIndex}
          onSelectOption={slash.executeOption}
        />

        {/* Note Mention Menu (`[[` popper) */}
        <NoteMentionMenu
          isOpen={noteMention.isOpen && !slash.isOpen}
          entries={noteMention.entries}
          selectedIndex={noteMention.selectedIndex}
          loading={noteMention.loading}
          loadErrorDetail={noteMention.loadErrorDetail}
          onSelect={noteMention.applyEntry}
          onHover={noteMention.setSelectedIndex}
        />

        {/* File Mention Menu (`@` popper) */}
        <FileMentionMenu
          isOpen={mention.isOpen && !slash.isOpen && !noteMention.isOpen}
          entries={mention.entries}
          noteEntries={mention.noteEntries}
          notesLoading={mention.notesLoading}
          noteLoadErrorDetail={mention.noteLoadErrorDetail}
          noteCapable={mention.noteCapable}
          skillEntries={mention.skillEntries}
          skillCapable={mention.skillCapable}
          agentEntries={mention.agentEntries}
          agentCapable={mention.agentCapable}
          selectedIndex={mention.selectedIndex}
          mode={mention.mode}
          dirPath={mention.dirPath}
          workingDir={workingDir ?? null}
          loading={mention.loading}
          error={mention.error}
          truncated={mention.truncated}
          hasFileQuery={mention.hasFileQuery}
          onSelect={mention.applyEntry}
          onSelectNote={mention.applyNote}
          onSelectSkill={mention.applySkill}
          onSelectAgent={mention.applyAgent}
          onHover={mention.setSelectedIndex}
        />

        {/* Quick Prompt Menu (`#` popper) */}
        <QuickPromptMenu
          isOpen={quickPrompt.isOpen && !slash.isOpen && !noteMention.isOpen && !mention.isOpen}
          entries={quickPrompt.entries}
          selectedIndex={quickPrompt.selectedIndex}
          query={quickPrompt.query}
          onSelect={quickPrompt.applyEntry}
          onHover={quickPrompt.setSelectedIndex}
        />

        {visibleTaskProgressSnapshot && (
          <TaskProgressPanel
            snapshot={visibleTaskProgressSnapshot}
            executionState={taskExecutionState}
            variant="embedded"
            onOpenWorkspace={onOpenWorkspace}
            workspaceOpen={workspacePanelVisible}
          />
        )}

        {/* Attached files preview (rendered above textarea) */}
        <AttachmentPreview
          attachedFiles={attachedFiles}
          onRemoveFile={onRemoveFile}
          onUpdateFile={onUpdateFile}
        />

        {/* Staged "quote to chat" references */}
        <AnimatedCollapse open={!!pendingQuotes?.length}>
          <div className="flex flex-wrap gap-1.5 px-3 pt-2">
            {pendingQuotes?.map((q, index) => {
              const lines =
                q.startLine === q.endLine ? `${q.startLine}` : `${q.startLine}-${q.endLine}`
              // Code-point-safe truncation so the preview can't split a surrogate
              // pair (emoji / astral CJK) and render a � at the cut.
              const cps = Array.from(q.content)
              const preview = cps.length > 400 ? `${cps.slice(0, 400).join("")}…` : q.content
              return (
                <span
                  key={`${q.path}:${lines}:${index}`}
                  className="inline-flex max-w-[260px] items-center gap-0.5 rounded-md border border-border/60 bg-secondary/40 py-0.5 pl-1 pr-1 text-xs text-foreground/80"
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
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
                    </TooltipTrigger>
                    {/* Hover preview of the quoted text (click jumps to it). */}
                    <TooltipContent side="top" className="max-w-[340px]">
                      <span className="block max-h-40 overflow-hidden whitespace-pre-wrap text-xs">
                        {preview}
                      </span>
                    </TooltipContent>
                  </Tooltip>
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

        {/* Pending send queue */}
        <AnimatedCollapse open={hasPendingQueue}>
          <div className="px-3 pt-2.5 pb-0 animate-in fade-in-0 slide-in-from-top-1 duration-200">
            <div className="rounded-lg border border-amber-500/20 bg-amber-500/8 px-2.5 py-2">
              <div className="mb-1.5 flex items-center gap-2">
                <BetweenHorizontalStart className="h-4 w-4 shrink-0 text-amber-500" />
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/80">
                  {t("chat.pendingQueueTitle", "待发送")} · {pendingQueueItems.length}
                </span>
                {pendingQueueItems.length > 2 && (
                  <IconTip
                    label={
                      pendingExpanded ? t("common.collapse", "收起") : t("common.expand", "展开")
                    }
                  >
                    <button
                      type="button"
                      onClick={() => setPendingExpanded((v) => !v)}
                      className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
                    >
                      {pendingExpanded ? (
                        <ChevronUp className="h-3.5 w-3.5" />
                      ) : (
                        <ChevronDown className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </IconTip>
                )}
              </div>
              <div className="flex flex-col gap-1.5">
                {pendingVisibleItems.map((item) => {
                  const edit = () =>
                    item.id === "__legacy__" ? onCancelPending?.() : onEditPending?.(item.id)
                  const discard = () =>
                    item.id === "__legacy__"
                      ? onDiscardPending?.()
                      : onDiscardPendingItem?.(item.id)
                  const readonly = item.status === "inserted"
                  const canCancelForce =
                    item.mode === "force_insert" && item.status === "waiting_tool_boundary"
                  return (
                    <div
                      key={item.id}
                      className="flex min-w-0 items-center gap-1.5 rounded-md bg-background/45 px-2 py-1.5"
                    >
                      <IconTip label={pendingStatusTip(item)}>
                        <span className="shrink-0 rounded-sm bg-amber-500/12 px-1.5 py-0.5 text-[11px] text-amber-700 dark:text-amber-300">
                          {pendingStatusLabel(item)}
                        </span>
                      </IconTip>
                      <span className="min-w-0 flex-1 truncate text-sm text-foreground/90">
                        {item.text}
                        {(item.attachmentCount > 0 || item.quoteCount > 0) && (
                          <span className="ml-1 text-xs text-muted-foreground">
                            +{item.attachmentCount + item.quoteCount}
                          </span>
                        )}
                      </span>
                      {canCancelForce ? (
                        <IconTip label={t("chat.pendingCancelForceInsert", "取消插入本轮")}>
                          <button
                            type="button"
                            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                            onClick={() => onCancelForceInsertPending?.(item.id)}
                          >
                            <Undo2 className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      ) : (
                        item.canForceInsert && (
                          <IconTip
                            label={t(
                              "chat.pendingForceInsertTip",
                              "会等正在执行的工具完成后插入给模型，不会打断当前工具。",
                            )}
                          >
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                              onClick={() => onForceInsertPending?.(item.id)}
                            >
                              <BetweenHorizontalStart className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
                        )
                      )}
                      {!readonly && (
                        <>
                          <IconTip label={t("chat.pendingEdit")}>
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                              onClick={edit}
                            >
                              <Pencil className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
                          <IconTip label={t("chat.pendingDelete")}>
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                              onClick={discard}
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
                        </>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          </div>
        </AnimatedCollapse>

        {/* Plan Mode Banner */}
        <AnimatedCollapse open={planState === "planning"}>
          <div
            className={`flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200${!hasVisibleTaskProgress && attachedFiles.length === 0 && !hasPendingQueue ? " rounded-t-2xl" : ""}`}
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

        {/* Tokenized composer: raw `input` stays plain text, selected mentions
            render as atomic chips inside the editable surface. */}
        <div className="relative">
          <MentionComposerInput
            ref={inputHandleRef}
            placeholder={
              planState === "planning"
                ? t("planMode.placeholder")
                : hasPendingQueue
                  ? t("chat.pendingQueued")
                  : t("chat.askAnything")
            }
            value={input}
            onChange={setComposerInput}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            onSelectionChange={() => {
              mention.recheckTrigger()
              noteMention.recheckTrigger()
              quickPrompt.recheckTrigger()
            }}
            workingDir={workingDir ?? null}
            fileEnabled={!!workingDir}
            noteEnabled={enableNoteMention}
            skillEnabled={enableSkillMention}
            agentMentionEnabled={enableAgentMention}
            agents={agents}
            hero={hero}
            readOnly={voice.state === "recording" || voice.state === "transcribing"}
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
          <AnimatedPresenceBox
            open={voice.state === "recording" || voice.state === "transcribing"}
            className="will-change-[opacity,transform]"
            enterClassName="translate-y-0 opacity-100"
            exitClassName="translate-y-1 opacity-0 pointer-events-none"
          >
            <RecordingBar
              transcribing={voice.state === "transcribing"}
              durationMs={voice.durationMs}
              levels={voice.levels}
              onCancel={handleVoiceCancel}
              onStop={() => void handleVoiceStop()}
            />
          </AnimatedPresenceBox>
        </AnimatedCollapse>
        <div
          style={
            normalToolbarOpen && toolbarMinHeight !== null
              ? { minHeight: toolbarMinHeight }
              : undefined
          }
        >
          <AnimatedCollapse open={normalToolbarOpen} overflow="visible-when-open">
            <div
              ref={toolbarRef}
              // Always two columns so Send/Stop stays pinned in its own column
              // on a single row — the left group wraps internally but never
              // pushes the send controls onto a line of their own.
              className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2 px-2 pb-2"
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

                {/* Model / Think / Temperature */}
                <ModelPicker
                  availableModels={availableModels}
                  activeModel={activeModel}
                  reasoningEffort={reasoningEffort}
                  onModelChange={onModelChange}
                  onEffortChange={onEffortChange}
                  currentModelInfo={currentModelInfo}
                  sessionTemperature={sessionTemperature}
                  onSessionTemperatureChange={onSessionTemperatureChange}
                />

                <AwarenessToggle sessionId={currentSessionId ?? null} disabled={incognitoEnabled} />

                {/* Knowledge Space attach + Plan toggle — primary actions, kept
                    inline down to the narrow `toolbarTight` tier (then they join
                    the "+" overflow menu, see renderOverflowMenuItems). */}
                {!toolbarTight && (
                  <KnowledgePicker
                    sessionId={currentSessionId ?? null}
                    projectId={projectId ?? null}
                    disabled={incognitoEnabled}
                    draftAttachments={draftKbAttachments}
                    onDraftAttachChange={onDraftKbAttachChange}
                  />
                )}

                {!toolbarTight && (
                  <IconTip label={planToggleTip}>
                    <button
                      aria-label={planToggleTip}
                      onClick={handlePlanToggle}
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
                )}

                {/* Tool Permission Mode — collapses into the "+" menu last, at
                    the narrowest tier (kept inline longer than Sandbox). */}
                {!permissionCollapsed && (
                  <PermissionModeSwitcher
                    permissionMode={permissionMode}
                    onPermissionModeChange={handlePermissionModeChange}
                  />
                )}
                {!sandboxCollapsed && (
                  <SandboxModeSwitcher
                    sandboxMode={sandboxMode}
                    onSandboxModeChange={onSandboxModeChange}
                  />
                )}
              </div>

              {/* Send & Stop — kept in its own column so toolbar wrapping never
              orphans the send button onto a half-empty row. */}
              <div className="flex min-h-8 min-w-[76px] shrink-0 items-center justify-end gap-1 self-end">
                <VoiceRecordButton
                  state={voice.state}
                  durationMs={voice.durationMs}
                  audioLevel={voice.audioLevel}
                  disabled={false}
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

                <IconTip
                  label={
                    loading && hasSendableContent && !sendDisabled
                      ? t("chat.queueMessage")
                      : t("chat.send")
                  }
                >
                  <Button
                    size="icon"
                    className="h-8 w-8 rounded-full shrink-0"
                    onClick={handleSend}
                    disabled={sendUnavailable}
                    aria-label={
                      loading && hasSendableContent && !sendDisabled
                        ? t("chat.queueMessage")
                        : t("chat.send")
                    }
                  >
                    <Send className="h-4 w-4" />
                  </Button>
                </IconTip>
              </div>
            </div>
          </AnimatedCollapse>
        </div>

        {/* Context-usage hairline fused into the dock's bottom border. */}
        {contextUsage ? <ContextUsageBottomBar usage={contextUsage} /> : null}
      </div>
    </div>
  )
}
