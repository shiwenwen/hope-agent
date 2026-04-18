import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { SlashCommandDef, CommandResult } from "./types"
import { CATEGORY_ORDER } from "./types"

export interface SlashCommandActions {
  /** Called when a command produces a CommandAction */
  onCommandAction: (result: CommandResult) => void
  /** Current session ID */
  sessionId: string | null
  /** Current agent ID */
  agentId: string
}

export interface UseSlashCommandsReturn {
  /** Whether the menu should be visible */
  isOpen: boolean
  /** Set menu open state (for button trigger) */
  setOpen: (open: boolean) => void
  /** Filtered and sorted commands */
  filteredCommands: SlashCommandDef[]
  /** Currently selected index */
  selectedIndex: number
  /** Handle keyboard events — returns true if consumed */
  handleKeyDown: (e: React.KeyboardEvent) => boolean
  /** Execute the currently selected command */
  executeSelected: () => void
  /** Execute a specific command by clicking */
  executeCommand: (cmd: SlashCommandDef) => void
  /** Whether a command is currently executing */
  executing: boolean
  /** Currently expanded command (showing arg options submenu) */
  expandedCmd: SlashCommandDef | null
  /** Filtered options for the expanded command */
  filteredOptions: string[]
  /** Selected option index in the submenu */
  selectedOptionIndex: number
  /** Execute a specific option from the submenu */
  executeOption: (cmd: SlashCommandDef, option: string) => void
}

/** Tag a skill passThrough result with the metadata ChatScreen needs to render
 *  the original `/skillname args` as the user bubble (via `displayText`). */
function annotateSkillPassThrough(
  result: CommandResult,
  cmd: SlashCommandDef,
  commandText: string,
  rawArgs: string,
): void {
  if (result.action?.type !== "passThrough" || cmd.category !== "skill") return
  result._isSkillPassThrough = true
  result._skillCommandText = commandText
  const trimmed = rawArgs.trim()
  if (trimmed) result._skillArgs = trimmed
}

export function useSlashCommands(
  input: string,
  setInput: (value: string) => void,
  actions: SlashCommandActions,
): UseSlashCommandsReturn {
  const [commands, setCommands] = useState<SlashCommandDef[]>([])
  const [isOpen, setIsOpen] = useState(false)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [executing, setExecuting] = useState(false)
  const [forceOpen, setForceOpen] = useState(false)
  const [expandedCmd, setExpandedCmd] = useState<SlashCommandDef | null>(null)
  const [selectedOptionIndex, setSelectedOptionIndex] = useState(0)
  const actionsRef = useRef(actions)
  actionsRef.current = actions

  // Load commands from backend (refresh when menu opens to pick up skill changes)
  const loadCommands = useCallback(() => {
    getTransport().call<SlashCommandDef[]>("list_slash_commands").then(setCommands).catch(() => {})
  }, [])

  useEffect(() => {
    loadCommands()
  }, [loadCommands])

  // Reload when menu is opened to catch skill changes
  useEffect(() => {
    if (forceOpen) {
      loadCommands()
    }
  }, [forceOpen, loadCommands])

  // Filter commands based on input
  const getFilterText = useCallback(() => {
    if (!input.startsWith("/")) return ""
    const spaceIdx = input.indexOf(" ")
    if (spaceIdx > 0) return "" // Already typing args, close menu
    return input.slice(1).toLowerCase()
  }, [input])

  const filteredCommands = useCallback(() => {
    // Button-triggered: show all commands (no input filter)
    if (forceOpen && !input.startsWith("/")) {
      return commands.toSorted((a, b) => {
        const ai = CATEGORY_ORDER.indexOf(a.category)
        const bi = CATEGORY_ORDER.indexOf(b.category)
        return ai - bi
      })
    }

    const filter = getFilterText()
    if (filter === "" && !input.startsWith("/")) return []

    const filtered = filter
      ? commands.filter(
          (c) => c.name.startsWith(filter) || c.name.includes(filter),
        )
      : commands

    // Sort by category order, then exact prefix first
    return filtered.toSorted((a, b) => {
      const ai = CATEGORY_ORDER.indexOf(a.category)
      const bi = CATEGORY_ORDER.indexOf(b.category)
      if (ai !== bi) return ai - bi
      if (filter) {
        const aExact = a.name.startsWith(filter) ? 0 : 1
        const bExact = b.name.startsWith(filter) ? 0 : 1
        if (aExact !== bExact) return aExact - bExact
      }
      return 0
    })
  }, [commands, getFilterText, input, forceOpen])()

  // Detect if input matches a command with argOptions (for auto-expanding submenu)
  const inputMatchCmd = useMemo(() => {
    if (!input.startsWith("/")) return null
    const spaceIdx = input.indexOf(" ")
    if (spaceIdx < 0) return null
    const name = input.slice(1, spaceIdx)
    return commands.find((c) => c.name === name && c.argOptions?.length) ?? null
  }, [input, commands])

  // Filter options for the expanded command based on typed input
  const filteredOptions = useMemo(() => {
    const cmd = expandedCmd ?? inputMatchCmd
    if (!cmd?.argOptions) return []
    const prefix = `/${cmd.name} `
    if (!input.startsWith(prefix)) return cmd.argOptions
    const typed = input.slice(prefix.length).toLowerCase()
    if (!typed) return cmd.argOptions
    return cmd.argOptions.filter((o) => o.startsWith(typed))
  }, [expandedCmd, inputMatchCmd, input])

  // Auto-expand submenu when user types "/<cmd-with-options> "
  useEffect(() => {
    if (inputMatchCmd && !expandedCmd) {
      setExpandedCmd(inputMatchCmd)
      setSelectedOptionIndex(0)
      setIsOpen(true)
    } else if (!inputMatchCmd && expandedCmd) {
      // Input no longer matches — close submenu
      setExpandedCmd(null)
    }
  }, [inputMatchCmd]) // eslint-disable-line react-hooks/exhaustive-deps

  // Reset option index when filtered options change
  useEffect(() => {
    setSelectedOptionIndex(0)
  }, [filteredOptions.length])

  // Determine if menu should be open
  const shouldBeOpen =
    forceOpen ||
    expandedCmd != null ||
    inputMatchCmd != null ||
    (input.startsWith("/") && input.indexOf(" ") < 0 && filteredCommands.length > 0)

  useEffect(() => {
    setIsOpen(shouldBeOpen)
    if (shouldBeOpen && !expandedCmd && !inputMatchCmd) {
      setSelectedIndex(0)
    }
  }, [shouldBeOpen]) // eslint-disable-line react-hooks/exhaustive-deps

  const executeCommandInner = useCallback(
    async (cmd: SlashCommandDef) => {
      // Build command text — when triggered by button (forceOpen, no "/" in input), no args from input
      const hasSlashInput = input.startsWith("/")
      const spaceIdx = hasSlashInput ? input.indexOf(" ") : -1
      const args = spaceIdx > 0 ? input.slice(spaceIdx + 1) : ""
      const commandText = `/${cmd.name}${args ? " " + args : ""}`

      setInput("")
      setIsOpen(false)
      setForceOpen(false)
      setExpandedCmd(null)
      setExecuting(true)

      try {
        const result = await getTransport().call<CommandResult>("execute_slash_command", {
          sessionId: actionsRef.current.sessionId,
          agentId: actionsRef.current.agentId,
          commandText,
        })
        annotateSkillPassThrough(result, cmd, commandText, args)
        actionsRef.current.onCommandAction(result)
      } catch (err) {
        actionsRef.current.onCommandAction({
          content: `Error: ${err}`,
          action: { type: "displayOnly" },
        })
      } finally {
        setExecuting(false)
      }
    },
    [input, setInput],
  )

  const executeOption = useCallback(
    (cmd: SlashCommandDef, option: string) => {
      setInput(`/${cmd.name} ${option}`)
      setExpandedCmd(null)
      setIsOpen(false)
      setForceOpen(false)
      setExecuting(true)

      const commandText = `/${cmd.name} ${option}`
      getTransport().call<CommandResult>("execute_slash_command", {
        sessionId: actionsRef.current.sessionId,
        agentId: actionsRef.current.agentId,
        commandText,
      })
        .then((result) => {
          annotateSkillPassThrough(result, cmd, commandText, option)
          actionsRef.current.onCommandAction(result)
        })
        .catch((err) =>
          actionsRef.current.onCommandAction({
            content: `Error: ${err}`,
            action: { type: "displayOnly" },
          }),
        )
        .finally(() => {
          setInput("")
          setExecuting(false)
        })
    },
    [setInput],
  )

  const executeSelected = useCallback(() => {
    if (expandedCmd && filteredOptions.length > 0) {
      executeOption(expandedCmd, filteredOptions[selectedOptionIndex])
      return
    }
    if (filteredCommands.length > 0 && selectedIndex < filteredCommands.length) {
      executeCommandInner(filteredCommands[selectedIndex])
    }
  }, [filteredCommands, selectedIndex, executeCommandInner, expandedCmd, filteredOptions, selectedOptionIndex, executeOption])

  const executeCommand = useCallback(
    (cmd: SlashCommandDef) => {
      if (cmd.argOptions?.length) {
        // Has built-in options: expand submenu
        setExpandedCmd(cmd)
        setSelectedOptionIndex(0)
        setInput(`/${cmd.name} `)
      } else if (cmd.hasArgs && !cmd.argsOptional) {
        // Has required args but no built-in options: fill in command and let user type
        setInput(`/${cmd.name} `)
        setIsOpen(false)
        setForceOpen(false)
      } else {
        // No args, or args are optional: execute directly
        executeCommandInner(cmd)
      }
    },
    [executeCommandInner, setInput],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent): boolean => {
      if (!isOpen) {
        // isOpen lags shouldBeOpen by one render. Intercept Enter, but resolve
        // via exact name match — selectedIndex is unreliable once a space in
        // input collapses the filter and reorders the list to start with /new.
        if (e.key === "Enter" && input.startsWith("/")) {
          const spaceIdx = input.indexOf(" ")
          const typedName = (spaceIdx > 0 ? input.slice(1, spaceIdx) : input.slice(1)).toLowerCase()
          if (typedName) {
            const match = commands.find((c) => c.name.toLowerCase() === typedName)
            if (match) {
              e.preventDefault()
              executeCommand(match)
              return true
            }
          }
        }
        return false
      }

      // When submenu is expanded, handle option navigation
      if (expandedCmd && filteredOptions.length > 0) {
        switch (e.key) {
          case "ArrowUp":
            e.preventDefault()
            setSelectedOptionIndex((prev) =>
              prev <= 0 ? filteredOptions.length - 1 : prev - 1,
            )
            return true
          case "ArrowDown":
            e.preventDefault()
            setSelectedOptionIndex((prev) =>
              prev >= filteredOptions.length - 1 ? 0 : prev + 1,
            )
            return true
          case "Tab": {
            // Tab: fill option into input box for further editing
            e.preventDefault()
            if (filteredOptions[selectedOptionIndex]) {
              setInput(`/${expandedCmd.name} ${filteredOptions[selectedOptionIndex]} `)
              setExpandedCmd(null)
              setIsOpen(false)
              setForceOpen(false)
            }
            return true
          }
          case "Enter":
            e.preventDefault()
            executeOption(expandedCmd, filteredOptions[selectedOptionIndex])
            return true
          case "Escape":
          case "ArrowLeft":
            e.preventDefault()
            setExpandedCmd(null)
            setInput("/")
            return true
          default:
            return false
        }
      }

      // Normal command list navigation
      switch (e.key) {
        case "ArrowUp":
          e.preventDefault()
          setSelectedIndex((prev) =>
            prev <= 0 ? filteredCommands.length - 1 : prev - 1,
          )
          return true

        case "ArrowDown":
          e.preventDefault()
          setSelectedIndex((prev) =>
            prev >= filteredCommands.length - 1 ? 0 : prev + 1,
          )
          return true

        case "Tab": {
          // Tab: autocomplete command name into input box
          e.preventDefault()
          const tabCmd = filteredCommands[selectedIndex]
          if (tabCmd) {
            if (tabCmd.argOptions?.length) {
              setExpandedCmd(tabCmd)
              setSelectedOptionIndex(0)
              setInput(`/${tabCmd.name} `)
            } else {
              setInput(`/${tabCmd.name} `)
              setIsOpen(false)
              setForceOpen(false)
            }
          }
          return true
        }
        case "Enter": {
          // Enter: execute command immediately
          e.preventDefault()
          const enterCmd = filteredCommands[selectedIndex]
          if (enterCmd) {
            executeCommand(enterCmd)
          }
          return true
        }

        case "Escape":
          e.preventDefault()
          setIsOpen(false)
          setExpandedCmd(null)
          // Only clear input if it was typed (starts with "/"), not button-triggered
          if (!forceOpen && input.startsWith("/")) {
            setInput("")
          }
          setForceOpen(false)
          return true

        default:
          return false
      }
    },
    [isOpen, filteredCommands, selectedIndex, executeCommand, setInput, forceOpen, input, expandedCmd, filteredOptions, selectedOptionIndex, executeOption],
  )

  const setOpen = useCallback(
    (open: boolean) => {
      if (open) {
        setForceOpen(true)
        setExpandedCmd(null)
      } else {
        setForceOpen(false)
        setIsOpen(false)
        setExpandedCmd(null)
      }
    },
    [],
  )

  return {
    isOpen,
    setOpen,
    filteredCommands,
    selectedIndex,
    handleKeyDown,
    executeSelected,
    executeCommand,
    executing,
    expandedCmd,
    filteredOptions,
    selectedOptionIndex,
    executeOption,
  }
}
