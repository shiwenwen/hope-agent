import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"

export type ThemeMode = "auto" | "light" | "dark"

function applyTheme(mode: ThemeMode) {
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
  invoke("set_window_theme", { isDark }).catch(() => {})
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>("auto")

  // Load theme from backend config.json on mount
  useEffect(() => {
    invoke<string>("get_theme")
      .then((stored) => {
        const mode = (stored === "light" || stored === "dark") ? stored : "auto"
        setThemeState(mode)
        applyTheme(mode)
      })
      .catch(() => {
        applyTheme("auto")
      })
  }, [])

  const setTheme = useCallback((mode: ThemeMode) => {
    setThemeState(mode)
    applyTheme(mode)
    // Persist to backend config.json
    invoke("set_theme", { theme: mode }).catch(() => {})
  }, [])

  // Listen for system changes when in "auto" mode
  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
    const handleChange = () => {
      if (theme === "auto") {
        applyTheme("auto")
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
