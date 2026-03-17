import { useState, useEffect, useCallback } from "react"

export type ThemeMode = "auto" | "light" | "dark"

const STORAGE_KEY = "theme-preference"

function getStoredTheme(): ThemeMode {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored === "auto" || stored === "light" || stored === "dark") {
      return stored
    }
  } catch {
    // localStorage not available
  }
  return "auto"
}

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
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>(getStoredTheme)

  const setTheme = useCallback((mode: ThemeMode) => {
    setThemeState(mode)
    try {
      localStorage.setItem(STORAGE_KEY, mode)
    } catch {
      // ignore
    }
    applyTheme(mode)
  }, [])

  // Apply theme on mount and listen for system changes when in "auto" mode
  useEffect(() => {
    applyTheme(theme)

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
    const handleChange = () => {
      // Only react to system changes in auto mode
      const current = getStoredTheme()
      if (current === "auto") {
        applyTheme("auto")
      }
    }

    mediaQuery.addEventListener("change", handleChange)
    return () => mediaQuery.removeEventListener("change", handleChange)
  }, [theme])

  // Cycle through modes: auto → light → dark → auto
  const cycleTheme = useCallback(() => {
    setTheme(
      theme === "auto" ? "light" : theme === "light" ? "dark" : "auto"
    )
  }, [theme, setTheme])

  return { theme, setTheme, cycleTheme }
}
