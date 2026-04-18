import { useEffect, useRef, useState, memo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { classifyWeather, generatePoints } from "./weatherUtils"
import type { WeatherData } from "./weatherUtils"
import WeatherCanvas from "./WeatherCanvas"
import ShootingStar from "./ShootingStar"
import CloudLayer from "./CloudLayer"
import WindStreaks from "./WindStreaks"

/**
 * AppBackground (formerly StarrySky)
 * Renders starry sky (dark mode) + real-time weather effects via Canvas.
 *
 * Weather types:
 *   - Clear/Sunny (WMO 0-1): golden glow + floating light motes
 *   - Cloudy (WMO 2-3): drifting CSS cloud shapes
 *   - Fog (WMO 45,48): layered translucent overlay
 *   - Rain/Drizzle (WMO 51-67, 80-82): canvas rain streaks
 *   - Snow (WMO 71-77, 85-86): canvas snowflakes
 *   - Thunderstorm (WMO 95-99): rain + lightning flash
 *   - Wind: affects particle angle when windSpeed > 30 km/h
 */

function AppBackgroundInner() {
  const [isDark, setIsDark] = useState(false)
  const [shootingStars, setShootingStars] = useState<number[]>([])
  const nextId = useRef(0)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const [uiEffectsEnabled, setUiEffectsEnabled] = useState(true)
  const [weatherCode, setWeatherCode] = useState<number | null>(null)
  const [windSpeed, setWindSpeed] = useState(0)

  const [points] = useState(() => ({
    starsSmall: generatePoints(200, 2000, 2000, 11),
    starsMedium: generatePoints(80, 2000, 2000, 29),
    starsLarge: generatePoints(30, 2000, 2000, 47),
  }))

  // Watch dark mode
  useEffect(() => {
    const root = document.documentElement
    const update = () => setIsDark(root.classList.contains("dark"))
    update()
    const observer = new MutationObserver(update)
    observer.observe(root, { attributes: true, attributeFilter: ["class"] })
    return () => observer.disconnect()
  }, [])

  // Reduced motion
  const [reducedMotion, setReducedMotion] = useState(() =>
    window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  )
  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)")
    const handler = (e: MediaQueryListEvent) => setReducedMotion(e.matches)
    mq.addEventListener("change", handler)
    return () => mq.removeEventListener("change", handler)
  }, [])

  // Load weather data
  useEffect(() => {
    let mounted = true

    const applyWeather = (w: WeatherData | null) => {
      if (!mounted) return
      if (w) {
        setWeatherCode(w.weatherCode)
        setWindSpeed(w.windSpeed ?? 0)
      } else {
        setWeatherCode(null)
        setWindSpeed(0)
      }
    }

    const loadData = async () => {
      try {
        const effects = await getTransport().call<boolean>("get_ui_effects_enabled")
        if (mounted) setUiEffectsEnabled(effects)
        if (effects) {
          try {
            const w = await getTransport().call<WeatherData | null>("get_current_weather")
            applyWeather(w)
          } catch {
            // weather might not be configured
          }
        }
      } catch (e) {
        console.error("Failed to load background effects data", e)
      }
    }
    loadData()

    const listener = () => loadData()
    const simulateListener = (e: Event) => {
      const customEvent = e as CustomEvent<{ weatherCode: number | null; windSpeed?: number }>
      const d = customEvent.detail
      setWeatherCode(d.weatherCode)
      setWindSpeed(d.windSpeed ?? 0)
    }

    window.addEventListener("ui-effects-changed", listener)
    window.addEventListener("simulate-weather", simulateListener)

    const unlistenWeather = getTransport().listen("weather-cache-updated", (payload) => {
      applyWeather(payload as WeatherData)
    })

    // Listen for config:changed from backend (e.g. oc-settings skill updates ui_effects)
    const unlistenConfig = getTransport().listen("config:changed", (raw) => {
      try {
        const payload = parsePayload<{ category?: string }>(raw)
        if (payload?.category === "ui_effects") {
          loadData()
        }
      } catch { /* ignore */ }
    })

    return () => {
      mounted = false
      window.removeEventListener("ui-effects-changed", listener)
      window.removeEventListener("simulate-weather", simulateListener)
      unlistenWeather()
      unlistenConfig()
    }
  }, [])

  // Shooting stars (dark mode, clear weather only)
  useEffect(() => {
    if (!uiEffectsEnabled || !isDark || reducedMotion) return

    const cleanupTimers: ReturnType<typeof setTimeout>[] = []
    const wType = weatherCode !== null ? classifyWeather(weatherCode) : null
    const hasWeather = wType !== null

    const scheduleNext = () => {
      if (hasWeather) {
        timerRef.current = setTimeout(scheduleNext, 30000)
        return
      }
      const delay = 6000 + Math.random() * 12000
      timerRef.current = setTimeout(() => {
        const id = nextId.current++
        setShootingStars((prev) => [...prev, id])
        const t = setTimeout(() => {
          setShootingStars((prev) => prev.filter((s) => s !== id))
        }, 2000)
        cleanupTimers.push(t)
        scheduleNext()
      }, delay)
    }

    scheduleNext()
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current)
      cleanupTimers.forEach(clearTimeout)
    }
  }, [uiEffectsEnabled, isDark, reducedMotion, weatherCode])

  if (!uiEffectsEnabled) return null

  const removeShootingStar = (id: number) => {
    setShootingStars((prev) => prev.filter((s) => s !== id))
  }

  const weatherType = weatherCode !== null ? classifyWeather(weatherCode) : null
  const isWindy = windSpeed > 30

  return (
    <div className="starry-sky-container" aria-hidden="true">
      {/* ── Starry Sky (Dark Mode) ── */}
      {isDark && (
        <>
          <div className="starry-layer starry-twinkle-1" style={{ boxShadow: points.starsSmall, width: 2, height: 2 }} />
          <div className="starry-layer starry-twinkle-2" style={{ boxShadow: points.starsMedium, width: 3, height: 3 }} />
          <div className="starry-layer starry-twinkle-3" style={{ boxShadow: points.starsLarge, width: 4, height: 4 }} />
          {shootingStars.map((id) => (
            <ShootingStar key={id} id={id} onDone={removeShootingStar} />
          ))}
        </>
      )}

      {/* ── Clouds ── */}
      {!reducedMotion && weatherType === "cloudy" && <CloudLayer count={6} />}

      {/* ── Fog ── */}
      {!reducedMotion && weatherType === "fog" && (
        <>
          <div className="weather-fog-overlay" />
          <CloudLayer count={8} isFog />
        </>
      )}

      {/* ── Wind Streaks ── */}
      {!reducedMotion && isWindy && <WindStreaks />}

      {/* ── Canvas Particles (rain, snow, sun motes, thunder) ── */}
      {!reducedMotion && weatherType && (
        <WeatherCanvas
          weatherType={weatherType}
          windSpeed={windSpeed}
          isDark={isDark}
        />
      )}
    </div>
  )
}

const AppBackground = memo(AppBackgroundInner)
export default AppBackground
