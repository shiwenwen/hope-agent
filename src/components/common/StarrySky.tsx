import { useEffect, useMemo, useRef, useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

/**
 * AppBackground (formerly StarrySky)
 * Renders the starry background for dark mode, and now
 * weather effects (rain, snow, clouds) regardless of theme.
 */

interface WeatherData {
  city: string
  weatherCode: number
  temperature: number
}

// Generate deterministic points via box-shadow strings
function generatePoints(count: number, maxX: number, maxY: number, seed = 1): string {
  let state = seed
  const next = () => {
    state = (state * 1664525 + 1013904223) >>> 0
    return state / 0xffffffff
  }
  const shadows: string[] = []
  for (let i = 0; i < count; i++) {
    const x = Math.round(next() * maxX)
    const y = Math.round(next() * maxY)
    shadows.push(`${x}px ${y}px currentColor`)
  }
  return shadows.join(", ")
}

// Shooting star component with random position and delay
function ShootingStar({ id, onDone }: { id: number; onDone: (id: number) => void }) {
  const style = useMemo(() => {
    const seeded = (offset: number) => {
      const value = Math.sin((id + 1) * 12.9898 + offset * 78.233) * 43758.5453
      return value - Math.floor(value)
    }
    const top = seeded(1) * 40 // top 0-40%
    const left = 50 + seeded(2) * 50 // right half of screen
    const duration = 0.8 + seeded(3) * 0.6 // 0.8-1.4s
    const trailWidth = 120 + seeded(4) * 180 // 120-300px trail length
    const travelDistance = trailWidth * 2.5 // travel proportional to trail
    return {
      top: `${top}%`,
      left: `${left}%`,
      animationDuration: `${duration}s`,
      width: `${trailWidth}px`,
      ["--travel" as string]: `-${travelDistance}px`,
    }
  }, [id])

  return (
    <div
      className="starry-shooting-star"
      style={style}
      onAnimationEnd={() => onDone(id)}
    />
  )
}

function AppBackgroundInner() {
  const [isDark, setIsDark] = useState(false)
  const [shootingStars, setShootingStars] = useState<number[]>([])
  const nextId = useRef(0)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const [uiEffectsEnabled, setUiEffectsEnabled] = useState(true)
  const [weatherCode, setWeatherCode] = useState<number | null>(null)

  // Points generated once for different layers
  const [points] = useState(() => ({
    starsSmall: generatePoints(200, 2000, 2000, 11),
    starsMedium: generatePoints(80, 2000, 2000, 29),
    starsLarge: generatePoints(30, 2000, 2000, 47),
    weather1: generatePoints(50, 2500, 2500, 99),
    weather2: generatePoints(50, 2500, 2500, 88),
  }))

  // Watch for .dark class changes
  useEffect(() => {
    const root = document.documentElement
    const update = () => setIsDark(root.classList.contains("dark"))
    update()

    const observer = new MutationObserver(update)
    observer.observe(root, { attributes: true, attributeFilter: ["class"] })
    return () => observer.disconnect()
  }, [])

  // Reduced motion preference
  const [reducedMotion, setReducedMotion] = useState(() =>
    window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  )
  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)")
    const handler = (e: MediaQueryListEvent) => setReducedMotion(e.matches)
    mq.addEventListener("change", handler)
    return () => mq.removeEventListener("change", handler)
  }, [])

  // Load effects config and weather
  useEffect(() => {
    let mounted = true
    const loadData = async () => {
      try {
        const effects = await invoke<boolean>("get_ui_effects_enabled")
        if (mounted) setUiEffectsEnabled(effects)

        if (effects) {
          try {
            const w = await invoke<WeatherData | null>("get_current_weather")
            if (mounted) {
              setWeatherCode(w ? w.weatherCode : null)
            }
          } catch(err) {
             // weather might not be initialized
             console.log("Weather fetch failed (possibly disabled)", err)
          }
        }
      } catch (e) {
        console.error("Failed to load background effects data", e)
      }
    }
    loadData()

    const listener = () => loadData()
    const simulateListener = (e: Event) => {
      const customEvent = e as CustomEvent<number | null>
      setWeatherCode(customEvent.detail)
    }

    window.addEventListener("ui-effects-changed", listener)
    window.addEventListener("simulate-weather", simulateListener)

    // Listen for backend weather cache updates (cold start, periodic refresh, force refresh)
    let unlistenWeather: (() => void) | null = null
    listen<WeatherData>("weather-cache-updated", (event) => {
      if (mounted) {
        setWeatherCode(event.payload.weatherCode)
      }
    }).then((fn) => {
      unlistenWeather = fn
    })

    return () => {
      mounted = false
      window.removeEventListener("ui-effects-changed", listener)
      window.removeEventListener("simulate-weather", simulateListener)
      unlistenWeather?.()
    }
  }, [])

  // Spawn shooting stars periodically
  useEffect(() => {
    if (!uiEffectsEnabled || !isDark || reducedMotion) return

    const cleanupTimers: ReturnType<typeof setTimeout>[] = []

    const scheduleNext = () => {
      // Less frequent shooting stars if it's bad weather!
      const isBadWeather = weatherCode !== null && (weatherCode > 1) 
      if (isBadWeather) {
         // Maybe just schedule it far into the future or not at all
         timerRef.current = setTimeout(scheduleNext, 30000)
         return
      }

      const delay = 6000 + Math.random() * 12000 // 6-18s
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

  // WMO Weather codes interpretation
  const isRain = weatherCode !== null && ((weatherCode >= 50 && weatherCode <= 69) || (weatherCode >= 80 && weatherCode <= 82) || (weatherCode >= 95))
  const isSnow = weatherCode !== null && ((weatherCode >= 71 && weatherCode <= 79) || weatherCode === 85 || weatherCode === 86)
  const isCloudy = weatherCode !== null && (weatherCode === 2 || weatherCode === 3)

  return (
    <div className="starry-sky-container" aria-hidden="true">
      {/* ── Starry Sky (Dark Mode Only) ── */}
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

      {/* ── Weather Effects (All themes) ── */}
      {!reducedMotion && isCloudy && (
        <div className="weather-cloud-layer" />
      )}
      
      {!reducedMotion && isRain && (
        <>
           <div className="weather-rain-layer weather-speed-1" style={{ boxShadow: points.weather1 }} />
           <div className="weather-rain-layer weather-speed-2" style={{ boxShadow: points.weather2, marginTop: '20vh', marginLeft: '50px' }} />
           <div className="weather-rain-layer weather-speed-3" style={{ boxShadow: points.weather1, marginTop: '50vh', marginLeft: '-50px' }} />
        </>
      )}

      {!reducedMotion && isSnow && (
        <>
           <div className="weather-snow-layer weather-snow-speed-1" style={{ boxShadow: points.weather1 }} />
           <div className="weather-snow-layer weather-snow-speed-2" style={{ boxShadow: points.weather2, marginTop: '30vh', marginLeft: '60px' }} />
           <div className="weather-snow-layer weather-snow-speed-3" style={{ boxShadow: points.weather1, marginTop: '60vh', marginLeft: '-60px' }} />
        </>
      )}
    </div>
  )
}

const AppBackground = memo(AppBackgroundInner)
export default AppBackground
