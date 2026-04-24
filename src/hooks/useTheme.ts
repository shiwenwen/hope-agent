import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"

export type ThemeMode = "auto" | "light" | "dark"

function normalizeTheme(raw: string | null | undefined): ThemeMode {
  return raw === "light" || raw === "dark" ? raw : "auto"
}

/** Apply theme visually (DOM + native window) without persisting to config */
export function applyThemeVisual(mode: ThemeMode) {
  const root = document.documentElement
  let isDark: boolean
  if (mode === "dark") {
    isDark = true
  } else if (mode === "light") {
    isDark = false
  } else {
    isDark = window.matchMedia("(prefers-color-scheme: dark)").matches
  }

  if (isDark) {
    root.classList.add("dark")
  } else {
    root.classList.remove("dark")
  }
  // Sync inline background to prevent flash on resize
  root.style.backgroundColor = isDark ? "#0f0f0f" : "#ffffff"
  root.style.colorScheme = isDark ? "dark" : "light"
  // Sync macOS NSWindow background color to match theme
  getTransport().call("set_window_theme", { isDark }).catch(() => {})
}

/** Apply theme visually and persist to backend config */
export function setThemePreference(mode: ThemeMode) {
  applyThemeVisual(mode)
  getTransport().call("set_theme", { theme: mode }).catch(() => {})
}

/** Load saved theme from backend config and apply it visually. */
export async function initThemeFromConfig(): Promise<ThemeMode> {
  try {
    const stored = await getTransport().call<string>("get_theme")
    const mode = normalizeTheme(stored)
    applyThemeVisual(mode)
    return mode
  } catch {
    applyThemeVisual("auto")
    return "auto"
  }
}

/** Listen for backend theme changes and keep DOM/native window in sync. */
export function listenThemeConfigChange(onChange?: (mode: ThemeMode) => void): () => void {
  return getTransport().listen("config:changed", (raw) => {
    try {
      const payload = parsePayload<{ category?: string }>(raw)
      if (payload?.category === "theme") {
        getTransport().call<string>("get_theme").then((stored) => {
          const mode = normalizeTheme(stored)
          onChange?.(mode)
          applyThemeVisual(mode)
        }).catch(() => {})
      }
    } catch {
      /* ignore parse errors */
    }
  })
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>("auto")

  // Load theme from backend config.json on mount (apply visually only, no write-back)
  useEffect(() => {
    initThemeFromConfig().then(setThemeState).catch(() => {})
  }, [])

  const setTheme = useCallback((mode: ThemeMode) => {
    setThemeState(mode)
    setThemePreference(mode)
  }, [])

  // Listen for config changes from backend (e.g. ha-settings skill updates theme)
  useEffect(() => {
    return listenThemeConfigChange(setThemeState)
  }, [])

  // Listen for system changes when in "auto" mode
  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
    const handleChange = () => {
      if (theme === "auto") {
        applyThemeVisual("auto")
      }
    }

    mediaQuery.addEventListener("change", handleChange)
    return () => mediaQuery.removeEventListener("change", handleChange)
  }, [theme])

  // Cycle through modes: auto → light → dark → auto
  const cycleTheme = useCallback(() => {
    setTheme(theme === "auto" ? "light" : theme === "light" ? "dark" : "auto")
  }, [theme, setTheme])

  return { theme, setTheme, cycleTheme }
}
