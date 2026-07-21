import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { FitAddon } from "@xterm/addon-fit"
import { Terminal } from "@xterm/xterm"
import "@xterm/xterm/css/xterm.css"
import "./terminal.css"
import { GripHorizontal, Maximize2, Minimize2, Plus, SquareTerminal, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"
import { getTransport } from "@/lib/transport-provider"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { IconTip } from "@/components/ui/tooltip"

export interface TerminalSummary {
  id: string
  cwd: string
  shell: string
  title: string
  createdAt: number
  status: "running" | "exited"
  exitCode: number | null
  cols: number
  rows: number
}

export interface TerminalSnapshot extends TerminalSummary {
  outputBase64: string
  seq: number
}

interface TerminalOutputEvent {
  terminalId?: string
  seq?: number
  dataBase64?: string
}

interface TerminalExitEvent {
  terminalId?: string
  exitCode?: number | null
}

interface TerminalPanelProps {
  open: boolean
  workingDir?: string | null
  onOpenChange: (open: boolean) => void
}

const HEIGHT_STORAGE_KEY = "hope-agent:terminal-panel-height"
const DEFAULT_HEIGHT = 270
const MIN_HEIGHT = 160
const MAX_HEIGHT_RATIO = 0.72

function maxHeightForViewport(viewportHeight: number): number {
  return Math.max(MIN_HEIGHT, Math.round(viewportHeight * MAX_HEIGHT_RATIO))
}

function clampHeight(height: number, viewportHeight: number): number {
  return Math.min(maxHeightForViewport(viewportHeight), Math.max(MIN_HEIGHT, height))
}

function readSavedHeight(): number {
  try {
    const raw = localStorage.getItem(HEIGHT_STORAGE_KEY)
    const value = raw === null ? DEFAULT_HEIGHT : Number(raw)
    const resolved = Number.isFinite(value) ? value : DEFAULT_HEIGHT
    return clampHeight(resolved, window.innerHeight)
  } catch {
    return clampHeight(DEFAULT_HEIGHT, window.innerHeight)
  }
}

function saveHeight(height: number) {
  try {
    localStorage.setItem(HEIGHT_STORAGE_KEY, String(Math.round(height)))
  } catch {
    // Storage is best-effort (private mode / quota).
  }
}

function mergeTerminal(
  terminals: TerminalSummary[],
  terminal: TerminalSummary,
): TerminalSummary[] {
  const index = terminals.findIndex((item) => item.id === terminal.id)
  if (index === -1) return [...terminals, terminal].sort((a, b) => a.createdAt - b.createdAt)
  return terminals.map((item, itemIndex) => (itemIndex === index ? terminal : item))
}

export function TerminalPanel({ open, workingDir, onOpenChange }: TerminalPanelProps) {
  const { t } = useTranslation()
  const [terminals, setTerminals] = useState<TerminalSummary[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [height, setHeight] = useState(readSavedHeight)
  const [viewportHeight, setViewportHeight] = useState(() => window.innerHeight)
  const [maximized, setMaximized] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [closingIds, setClosingIds] = useState<Set<string>>(() => new Set())

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<TerminalSummary[]>("terminal_list")
      .then((items) => {
        if (cancelled) return
        setTerminals(items)
        setActiveId((current) => current ?? items.at(-1)?.id ?? null)
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason))
      })
      .finally(() => {
        if (!cancelled) setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const transport = getTransport()
    const offCreated = transport.listen("terminal:created", (raw) => {
      const terminal = (raw as { terminal?: TerminalSnapshot })?.terminal
      if (!terminal) return
      setTerminals((current) => mergeTerminal(current, terminal))
      setActiveId((current) => current ?? terminal.id)
    })
    const offExit = transport.listen("terminal:exit", (raw) => {
      const event = raw as TerminalExitEvent
      if (!event.terminalId) return
      setTerminals((current) =>
        current.map((terminal) =>
          terminal.id === event.terminalId
            ? { ...terminal, status: "exited", exitCode: event.exitCode ?? null }
            : terminal,
        ),
      )
    })
    const offClosed = transport.listen("terminal:closed", (raw) => {
      const id = (raw as { terminalId?: string })?.terminalId
      if (!id) return
      setTerminals((current) => current.filter((terminal) => terminal.id !== id))
      setActiveId((current) => (current === id ? null : current))
    })
    return () => {
      offCreated()
      offExit()
      offClosed()
    }
  }, [])

  const createTerminal = useCallback(async () => {
    if (creating) return
    setCreating(true)
    setError(null)
    try {
      const terminal = await getTransport().call<TerminalSnapshot>("terminal_create", {
        request: { cwd: workingDir || null, cols: 100, rows: 28 },
      })
      setTerminals((current) => mergeTerminal(current, terminal))
      setActiveId(terminal.id)
    } catch (reason) {
      logger.error("ui", "TerminalPanel::create", "Failed to create terminal", reason)
      setError(String(reason))
    } finally {
      setCreating(false)
    }
  }, [creating, workingDir])

  useEffect(() => {
    if (
      open &&
      loaded &&
      terminals.length === 0 &&
      !creating &&
      closingIds.size === 0 &&
      !error
    ) {
      void createTerminal()
    }
  }, [closingIds.size, createTerminal, creating, error, loaded, open, terminals.length])

  useEffect(() => {
    if (activeId && terminals.some((terminal) => terminal.id === activeId)) return
    setActiveId(terminals.at(-1)?.id ?? null)
  }, [activeId, terminals])

  useEffect(() => {
    const handleWindowResize = () => {
      const nextViewportHeight = window.innerHeight
      setViewportHeight(nextViewportHeight)
      setHeight((current) => clampHeight(current, nextViewportHeight))
    }
    window.addEventListener("resize", handleWindowResize)
    return () => window.removeEventListener("resize", handleWindowResize)
  }, [])

  const closeTerminal = useCallback(
    async (terminalId: string) => {
      if (closingIds.has(terminalId)) return
      const wasLastTerminal = terminals.length === 1
      setClosingIds((current) => new Set(current).add(terminalId))
      setError(null)
      try {
        await getTransport().call("terminal_close", { terminalId })
        const refreshed = await getTransport()
          .call<TerminalSummary[]>("terminal_list")
          .catch(() => null)
        if (refreshed) {
          setTerminals(refreshed)
          setActiveId((current) =>
            current && refreshed.some((terminal) => terminal.id === current)
              ? current
              : (refreshed.at(-1)?.id ?? null),
          )
          if (refreshed.length === 0) onOpenChange(false)
        } else {
          setTerminals((current) => current.filter((terminal) => terminal.id !== terminalId))
          setActiveId((current) => (current === terminalId ? null : current))
          if (wasLastTerminal) onOpenChange(false)
        }
      } catch (reason) {
        logger.warn("ui", "TerminalPanel::close", "Failed to close terminal", reason)
        setError(String(reason))
      } finally {
        setClosingIds((current) => {
          const next = new Set(current)
          next.delete(terminalId)
          return next
        })
      }
    },
    [closingIds, onOpenChange, terminals.length],
  )

  const beginResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (maximized) return
      event.preventDefault()
      const startY = event.clientY
      const startHeight = height
      const maxHeight = maxHeightForViewport(viewportHeight)
      setDragging(true)

      const move = (pointerEvent: PointerEvent) => {
        setHeight(
          Math.min(maxHeight, Math.max(MIN_HEIGHT, startHeight + startY - pointerEvent.clientY)),
        )
      }
      const end = () => {
        setDragging(false)
        window.removeEventListener("pointermove", move)
        window.removeEventListener("pointerup", end)
        document.body.style.removeProperty("user-select")
        document.body.style.removeProperty("cursor")
      }
      document.body.style.userSelect = "none"
      document.body.style.cursor = "ns-resize"
      window.addEventListener("pointermove", move)
      window.addEventListener("pointerup", end, { once: true })
    },
    [height, maximized, viewportHeight],
  )

  useEffect(() => {
    if (!dragging) saveHeight(height)
  }, [dragging, height])

  const visibleHeight = maximized
    ? maxHeightForViewport(viewportHeight)
    : clampHeight(height, viewportHeight)
  const activeTerminal = useMemo(
    () => terminals.find((terminal) => terminal.id === activeId) ?? null,
    [activeId, terminals],
  )

  return (
    <section
      className={cn(
        "relative shrink-0 overflow-hidden bg-background",
        open && "border-t border-border",
        !dragging && "transition-[height] duration-150 ease-out",
      )}
      style={{ height: open ? visibleHeight : 0 }}
      aria-label={t("terminal.title", "终端")}
      aria-hidden={!open}
    >
      {open ? (
        <>
          <div
            className="group absolute inset-x-0 top-0 z-20 flex h-2 cursor-ns-resize items-start justify-center"
            onPointerDown={beginResize}
            role="separator"
            aria-orientation="horizontal"
            aria-label={t("terminal.resize", "调整终端高度")}
          >
            <GripHorizontal className="mt-0.5 h-3.5 w-3.5 text-muted-foreground/0 transition-colors group-hover:text-muted-foreground/70" />
          </div>

          <header className="flex h-9 items-center border-b border-border/70 bg-muted/20 pl-2 pr-1">
            <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
              {terminals.map((terminal) => {
                const active = terminal.id === activeId
                return (
                  <div
                    key={terminal.id}
                    className={cn(
                      "group/tab flex h-7 max-w-[220px] shrink-0 items-center gap-1.5 rounded-md px-2 text-[11px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground",
                      active && "bg-secondary/70 text-foreground",
                    )}
                    data-ha-title-tip={`${terminal.shell} — ${terminal.cwd}`}
                  >
                    <button
                      type="button"
                      onClick={() => setActiveId(terminal.id)}
                      className="flex min-w-0 flex-1 items-center gap-1.5"
                    >
                      <SquareTerminal className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">
                        {terminal.title} · {basename(terminal.cwd)}
                      </span>
                      {terminal.status === "exited" ? (
                        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-muted-foreground/45" />
                      ) : null}
                    </button>
                    <button
                      type="button"
                      aria-label={t("terminal.closeTab", "关闭终端")}
                      disabled={closingIds.has(terminal.id)}
                      className="ml-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded text-muted-foreground/60 opacity-0 transition-opacity hover:bg-secondary hover:text-foreground disabled:pointer-events-none disabled:opacity-30 group-hover/tab:opacity-100"
                      onClick={() => void closeTerminal(terminal.id)}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                )
              })}
              <IconTip label={t("terminal.new", "新建终端")}>
                <button
                  type="button"
                  onClick={() => void createTerminal()}
                  disabled={creating}
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:opacity-40"
                  aria-label={t("terminal.new", "新建终端")}
                >
                  <Plus className={cn("h-4 w-4", creating && "animate-pulse")} />
                </button>
              </IconTip>
            </div>

            {activeTerminal?.status === "exited" ? (
              <span className="mr-2 shrink-0 text-[10px] text-muted-foreground">
                {t("terminal.exited", "已退出")} {activeTerminal.exitCode ?? ""}
              </span>
            ) : null}
            <IconTip
              label={
                maximized ? t("terminal.restore", "还原终端") : t("terminal.maximize", "最大化终端")
              }
            >
              <button
                type="button"
                onClick={() => setMaximized((value) => !value)}
                className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
                aria-label={
                  maximized
                    ? t("terminal.restore", "还原终端")
                    : t("terminal.maximize", "最大化终端")
                }
              >
                {maximized ? (
                  <Minimize2 className="h-3.5 w-3.5" />
                ) : (
                  <Maximize2 className="h-3.5 w-3.5" />
                )}
              </button>
            </IconTip>
            <IconTip label={t("terminal.hide", "隐藏终端")}>
              <button
                type="button"
                onClick={() => onOpenChange(false)}
                className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
                aria-label={t("terminal.hide", "隐藏终端")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          </header>

          <div className="relative h-[calc(100%-2.25rem)] bg-[color:var(--terminal-background)]">
            {terminals.map((terminal) => (
              <TerminalView
                key={terminal.id}
                terminalId={terminal.id}
                active={terminal.id === activeId}
              />
            ))}
            {error ? (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-6 text-center text-xs text-muted-foreground">
                <span>{error}</span>
                <button
                  type="button"
                  onClick={() => {
                    setError(null)
                    if (terminals.length === 0) void createTerminal()
                  }}
                  className="rounded-md bg-secondary/70 px-2.5 py-1 text-foreground hover:bg-secondary"
                >
                  {t("common.retry", "重试")}
                </button>
              </div>
            ) : null}
          </div>
        </>
      ) : null}
    </section>
  )
}

function TerminalView({ terminalId, active }: { terminalId: string; active: boolean }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const terminal = new Terminal({
      allowProposedApi: false,
      allowTransparency: true,
      cursorBlink: true,
      cursorStyle: "bar",
      fontFamily: '"SFMono-Regular", "SF Mono", Menlo, Monaco, Consolas, monospace',
      fontSize: 13,
      fontWeight: "400",
      fontWeightBold: "600",
      lineHeight: 1.3,
      scrollback: 10_000,
      theme: terminalTheme(),
    })
    const fit = new FitAddon()
    terminal.loadAddon(fit)
    terminal.open(container)
    terminalRef.current = terminal
    fitRef.current = fit

    let disposed = false
    let initialized = false
    let lastSeq = 0
    let queuedOutput: TerminalOutputEvent[] = []
    let inputBuffer = ""
    let inputTimer: ReturnType<typeof setTimeout> | null = null
    let writeChain = Promise.resolve()
    let resizeTimer: ReturnType<typeof setTimeout> | null = null
    let snapshotGeneration = 0

    const writeBytes = (dataBase64: string) => {
      if (!dataBase64) return
      try {
        terminal.write(decodeBase64(dataBase64))
      } catch (reason) {
        logger.warn("ui", "TerminalView::decode", "Invalid terminal output", reason)
      }
    }

    const applySnapshot = (snapshot: TerminalSnapshot) => {
      if (disposed) return
      terminal.reset()
      writeBytes(snapshot.outputBase64)
      lastSeq = snapshot.seq
      initialized = true
      const pending = queuedOutput.sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0))
      queuedOutput = []
      for (const event of pending) {
        if ((event.seq ?? 0) <= lastSeq) continue
        writeBytes(event.dataBase64 ?? "")
        lastSeq = event.seq ?? lastSeq
      }
    }

    const syncSnapshot = () => {
      const generation = ++snapshotGeneration
      void getTransport()
        .call<TerminalSnapshot>("terminal_snapshot", { terminalId })
        .then((snapshot) => {
          if (generation !== snapshotGeneration) return
          applySnapshot(snapshot)
        })
        .catch((reason) => {
          if (!disposed && generation === snapshotGeneration) {
            terminal.writeln(`\r\n\x1b[31m[Hope Agent] ${String(reason)}\x1b[0m`)
          }
        })
    }

    const offOutput = getTransport().listen("terminal:output", (raw) => {
      const event = raw as TerminalOutputEvent
      if (event.terminalId !== terminalId || !event.dataBase64) return
      if (!initialized) {
        queuedOutput.push(event)
        return
      }
      const seq = event.seq ?? 0
      if (seq <= lastSeq) return
      if (lastSeq > 0 && seq > lastSeq + 1) {
        initialized = false
        queuedOutput = [event]
        syncSnapshot()
        return
      }
      writeBytes(event.dataBase64)
      lastSeq = seq
    })
    const offResync = getTransport().listen(TRANSPORT_EVENT_RESYNC_REQUIRED, () => {
      initialized = false
      queuedOutput = []
      syncSnapshot()
    })
    const offTerminalResync = getTransport().listen("terminal:resync_required", () => {
      initialized = false
      queuedOutput = []
      syncSnapshot()
    })

    const flushInput = () => {
      inputTimer = null
      const data = inputBuffer
      inputBuffer = ""
      if (!data) return
      writeChain = writeChain
        .then(() => getTransport().call<void>("terminal_write", { terminalId, data }))
        .catch((reason) => {
          logger.warn("ui", "TerminalView::input", "Failed to write terminal input", reason)
        })
    }
    const inputDisposable = terminal.onData((data) => {
      inputBuffer += data
      if (data.includes("\r") || inputBuffer.length >= 1024) {
        if (inputTimer) clearTimeout(inputTimer)
        flushInput()
      } else if (!inputTimer) {
        inputTimer = setTimeout(flushInput, 16)
      }
    })

    const scheduleResize = (cols: number, rows: number) => {
      if (resizeTimer) clearTimeout(resizeTimer)
      resizeTimer = setTimeout(() => {
        void getTransport()
          .call("terminal_resize", { terminalId, cols, rows })
          .catch(() => {})
      }, 40)
    }
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      scheduleResize(cols, rows)
    })
    const fitAndResize = () => {
      if (disposed || !container.offsetParent) return
      try {
        fit.fit()
      } catch {
        // A hidden/zero-size panel is expected during transitions.
      }
    }
    const resizeObserver = new ResizeObserver(fitAndResize)
    resizeObserver.observe(container)
    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = terminalTheme()
    })
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    })

    syncSnapshot()
    requestAnimationFrame(fitAndResize)

    return () => {
      disposed = true
      if (inputTimer) clearTimeout(inputTimer)
      if (resizeTimer) clearTimeout(resizeTimer)
      flushInput()
      offOutput()
      offResync()
      offTerminalResync()
      inputDisposable.dispose()
      resizeDisposable.dispose()
      resizeObserver.disconnect()
      themeObserver.disconnect()
      terminal.dispose()
      terminalRef.current = null
      fitRef.current = null
    }
  }, [terminalId])

  useEffect(() => {
    if (!active) return
    const frame = requestAnimationFrame(() => {
      try {
        fitRef.current?.fit()
      } catch {
        // A hidden/zero-size panel is expected during transitions.
      }
      terminalRef.current?.focus()
    })
    return () => cancelAnimationFrame(frame)
  }, [active])

  return (
    <div
      ref={containerRef}
      className={cn("hope-terminal absolute inset-0", !active && "hidden")}
      aria-hidden={!active}
    />
  )
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}

function terminalTheme() {
  const dark = document.documentElement.classList.contains("dark")
  const appStyle = getComputedStyle(document.body)
  const background = opaqueColor(
    appStyle.backgroundColor,
    dark ? "rgb(18, 18, 18)" : "rgb(255, 255, 255)",
  )
  const foreground = opaqueColor(
    appStyle.color,
    dark ? "rgb(230, 230, 230)" : "rgb(18, 18, 18)",
  )
  return dark
    ? {
        background,
        foreground,
        cursor: foreground,
        cursorAccent: background,
        selectionBackground: "#3f4854",
        selectionInactiveBackground: "#303640",
        black: "#1e1e1e",
        red: "#f14c4c",
        green: "#23d18b",
        yellow: "#f5f543",
        blue: "#3b8eea",
        magenta: "#d670d6",
        cyan: "#29b8db",
        white: "#e5e5e5",
        brightBlack: "#8c959f",
        brightRed: "#f14c4c",
        brightGreen: "#23d18b",
        brightYellow: "#f5f543",
        brightBlue: "#3b8eea",
        brightMagenta: "#d670d6",
        brightCyan: "#29b8db",
        brightWhite: "#ffffff",
      }
    : {
        background,
        foreground,
        cursor: foreground,
        cursorAccent: background,
        selectionBackground: "#b6d7ff",
        selectionInactiveBackground: "#d8e9ff",
        black: "#1f2328",
        red: "#cf222e",
        green: "#116329",
        yellow: "#4d2d00",
        blue: "#0969da",
        magenta: "#8250df",
        cyan: "#1b7c83",
        white: "#6e7781",
        brightBlack: "#57606a",
        brightRed: "#a40e26",
        brightGreen: "#1a7f37",
        brightYellow: "#633c01",
        brightBlue: "#218bff",
        brightMagenta: "#a475f9",
        brightCyan: "#3192aa",
        brightWhite: "#8c959f",
      }
}

function opaqueColor(value: string, fallback: string): string {
  const normalized = value.trim().toLowerCase()
  return !normalized || normalized === "transparent" || normalized === "rgba(0, 0, 0, 0)"
    ? fallback
    : value
}
