import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"

export type ThemeMode = "auto" | "light" | "dark"

/** Apply theme visually (DOM + native window) without persisting to config */
function applyThemeVisual(mode: ThemeMode) {
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
function applyTheme(mode: ThemeMode) {
  applyThemeVisual(mode)
  getTransport().call("set_theme", { theme: mode }).catch(() => {})
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>("auto")

  // Load theme from backend config.json on mount (apply visually only, no write-back)
  useEffect(() => {
    getTransport().call<string>("get_theme")
      .then((stored) => {
        const mode = (stored === "light" || stored === "dark") ? stored : "auto"
        setThemeState(mode)
        applyThemeVisual(mode)
      })
      .catch(() => {
        applyThemeVisual("auto")
      })
  }, [])

  const setTheme = useCallback((mode: ThemeMode) => {
    setThemeState(mode)
    applyTheme(mode)
  }, [])

  // Listen for config changes from backend (e.g. oc-settings skill updates theme)
  useEffect(() => {
    return getTransport().listen("config:changed", (raw) => {
      try {
        const payload = typeof raw === "string" ? JSON.parse(raw) : raw
        if (payload?.category === "theme") {
          getTransport().call<string>("get_theme").then((stored) => {
            const mode = (stored === "light" || stored === "dark") ? stored : "auto"
            setThemeState(mode)
            applyThemeVisual(mode)
          }).catch(() => {})
        }
      } catch { /* ignore parse errors */ }
    })
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
