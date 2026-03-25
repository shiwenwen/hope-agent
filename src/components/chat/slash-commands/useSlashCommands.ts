import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
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
  const actionsRef = useRef(actions)
  actionsRef.current = actions

  // Load commands from backend (refresh when menu opens to pick up skill changes)
  const loadCommands = useCallback(() => {
    invoke<SlashCommandDef[]>("list_slash_commands").then(setCommands).catch(() => {})
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

  // Determine if menu should be open
  const shouldBeOpen =
    forceOpen ||
    (input.startsWith("/") && input.indexOf(" ") < 0 && filteredCommands.length > 0)

  useEffect(() => {
    setIsOpen(shouldBeOpen)
    if (shouldBeOpen) {
      setSelectedIndex(0)
    }
  }, [shouldBeOpen])

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
      setExecuting(true)

      try {
        const result = await invoke<CommandResult>("execute_slash_command", {
          sessionId: actionsRef.current.sessionId,
          agentId: actionsRef.current.agentId,
          commandText,
        })
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

  const executeSelected = useCallback(() => {
    if (filteredCommands.length > 0 && selectedIndex < filteredCommands.length) {
      executeCommandInner(filteredCommands[selectedIndex])
    }
  }, [filteredCommands, selectedIndex, executeCommandInner])

  const executeCommand = useCallback(
    (cmd: SlashCommandDef) => {
      if (cmd.hasArgs) {
        // For commands with args, fill in the command and let user type args
        setInput(`/${cmd.name} `)
        setIsOpen(false)
        setForceOpen(false)
      } else {
        executeCommandInner(cmd)
      }
    },
    [executeCommandInner, setInput],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent): boolean => {
      if (!isOpen) return false

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

        case "Tab":
        case "Enter": {
          e.preventDefault()
          const cmd = filteredCommands[selectedIndex]
          if (cmd) {
            executeCommand(cmd)
          }
          return true
        }

        case "Escape":
          e.preventDefault()
          setIsOpen(false)
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
    [isOpen, filteredCommands, selectedIndex, executeCommand, setInput, forceOpen, input],
  )

  const setOpen = useCallback(
    (open: boolean) => {
      if (open) {
        setForceOpen(true)
      } else {
        setForceOpen(false)
        setIsOpen(false)
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
  }
}
