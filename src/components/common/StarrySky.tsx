import { useEffect, useMemo, useRef, useState, memo, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

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

interface WeatherData {
  city: string
  weatherCode: number
  temperature: number
  windSpeed: number
  humidity: number
  weatherDescription: string
  locationName: string
}

type WeatherType = "cloudy" | "fog" | "rain" | "snow" | "thunder" | null

function classifyWeather(code: number): WeatherType {
  if (code >= 95) return "thunder"
  if ((code >= 51 && code <= 67) || (code >= 80 && code <= 82)) return "rain"
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "snow"
  if (code === 45 || code === 48) return "fog"
  if (code === 2 || code === 3) return "cloudy"
  if (code <= 1) return null
  return null
}

// ────────────────────────────────────────────
// Canvas Weather Particle System
// ────────────────────────────────────────────

interface RainDrop {
  x: number
  y: number
  speed: number
  length: number
  opacity: number
}

interface SnowFlake {
  x: number
  y: number
  speed: number
  radius: number
  opacity: number
  swayOffset: number
  swaySpeed: number
  rotation: number
  rotationSpeed: number
  branches: number // 5 or 6
}


function WeatherCanvas({
  weatherType,
  windSpeed,
  isDark,
}: {
  weatherType: WeatherType
  windSpeed: number
  isDark: boolean
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const animRef = useRef<number>(0)
  const particlesRef = useRef<{
    rain: RainDrop[]
    snow: SnowFlake[]
  }>({ rain: [], snow: [] })
  const flashRef = useRef({ active: false, opacity: 0, nextFlash: 0 })

  const initParticles = useCallback(
    (w: number, h: number) => {
      const rain: RainDrop[] = []
      const snow: SnowFlake[] = []

      if (weatherType === "rain" || weatherType === "thunder") {
        const count = weatherType === "thunder" ? 220 : 160
        for (let i = 0; i < count; i++) {
          rain.push({
            x: Math.random() * (w + 200) - 100,
            y: Math.random() * h,
            speed: 12 + Math.random() * 10,
            length: 18 + Math.random() * 22,
            opacity: 0.15 + Math.random() * 0.35,
          })
        }
      }

      if (weatherType === "snow") {
        for (let i = 0; i < 100; i++) {
          snow.push({
            x: Math.random() * w,
            y: Math.random() * h,
            speed: 0.4 + Math.random() * 1.2,
            radius: 2 + Math.random() * 5,
            opacity: 0.25 + Math.random() * 0.45,
            swayOffset: Math.random() * Math.PI * 2,
            swaySpeed: 0.3 + Math.random() * 0.7,
            rotation: Math.random() * Math.PI * 2,
            rotationSpeed: (Math.random() - 0.5) * 0.02,
            branches: Math.random() > 0.3 ? 6 : 5,
          })
        }
      }

      particlesRef.current = { rain, snow }
    },
    [weatherType],
  )

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext("2d")
    if (!ctx) return

    let w = window.innerWidth
    let h = window.innerHeight
    canvas.width = w
    canvas.height = h

    initParticles(w, h)

    const handleResize = () => {
      w = window.innerWidth
      h = window.innerHeight
      canvas.width = w
      canvas.height = h
      initParticles(w, h)
    }
    window.addEventListener("resize", handleResize)

    // Wind angle: wind > 30 km/h tilts particles
    const windAngle = Math.min(windSpeed / 80, 0.5) // max ~30 degree tilt
    const windDx = Math.sin(windAngle)

    let time = 0
    const flash = flashRef.current

    const animate = () => {
      ctx.clearRect(0, 0, w, h)
      time += 0.016 // ~60fps
      const { rain, snow } = particlesRef.current

      // ── Rain ──
      if (weatherType === "rain" || weatherType === "thunder") {
        rain.forEach((d) => {
          d.y += d.speed
          d.x += d.speed * windDx

          if (d.y > h + 10) {
            d.y = -d.length - Math.random() * 60
            d.x = Math.random() * (w + 200) - 100
          }
          if (d.x > w + 100) d.x = -100
          if (d.x < -100) d.x = w + 100

          const endX = d.x + d.length * windDx * 0.5
          const endY = d.y + d.length

          ctx.beginPath()
          ctx.moveTo(d.x, d.y)
          ctx.lineTo(endX, endY)
          ctx.strokeStyle = isDark
            ? `rgba(170, 200, 255, ${d.opacity})`
            : `rgba(80, 120, 200, ${d.opacity * 0.7})`
          ctx.lineWidth = 1.5
          ctx.stroke()
        })
      }

      // ── Snow ──
      if (weatherType === "snow") {
        snow.forEach((f) => {
          f.y += f.speed
          f.x += Math.sin(time * f.swaySpeed + f.swayOffset) * 0.6
          f.x += windDx * f.speed * 2
          f.rotation += f.rotationSpeed

          if (f.y > h + 10) {
            f.y = -f.radius * 2 - Math.random() * 40
            f.x = Math.random() * w
          }
          if (f.x > w + 20) f.x = -20
          if (f.x < -20) f.x = w + 20

          ctx.save()
          ctx.translate(f.x, f.y)
          ctx.rotate(f.rotation)
          ctx.strokeStyle = isDark
            ? `rgba(255, 255, 255, ${f.opacity})`
            : `rgba(160, 175, 200, ${f.opacity * 0.85})`
          ctx.lineWidth = f.radius > 4 ? 1.2 : 0.8
          ctx.lineCap = "round"

          const r = f.radius
          const n = f.branches
          // Draw snowflake: n main branches with sub-branches
          for (let b = 0; b < n; b++) {
            const angle = (b / n) * Math.PI * 2
            const cos = Math.cos(angle)
            const sin = Math.sin(angle)

            // Main branch
            ctx.beginPath()
            ctx.moveTo(0, 0)
            ctx.lineTo(cos * r, sin * r)
            ctx.stroke()

            // Sub-branches (only for larger flakes)
            if (r > 3) {
              const subLen = r * 0.35
              const branchPoint = 0.55
              const bx = cos * r * branchPoint
              const by = sin * r * branchPoint
              const subAngle1 = angle + 0.5
              const subAngle2 = angle - 0.5

              ctx.beginPath()
              ctx.moveTo(bx, by)
              ctx.lineTo(bx + Math.cos(subAngle1) * subLen, by + Math.sin(subAngle1) * subLen)
              ctx.stroke()

              ctx.beginPath()
              ctx.moveTo(bx, by)
              ctx.lineTo(bx + Math.cos(subAngle2) * subLen, by + Math.sin(subAngle2) * subLen)
              ctx.stroke()
            }
          }
          ctx.restore()
        })
      }

      // ── Thunder flash ──
      if (weatherType === "thunder") {
        if (!flash.active && time > flash.nextFlash) {
          flash.active = true
          flash.opacity = 0.25 + Math.random() * 0.15
          flash.nextFlash = time + 4 + Math.random() * 8
        }
        if (flash.active) {
          ctx.fillStyle = `rgba(255, 255, 255, ${flash.opacity})`
          ctx.fillRect(0, 0, w, h)
          flash.opacity -= 0.02
          if (flash.opacity <= 0) flash.active = false
        }
      }

      animRef.current = requestAnimationFrame(animate)
    }

    animRef.current = requestAnimationFrame(animate)

    return () => {
      cancelAnimationFrame(animRef.current)
      window.removeEventListener("resize", handleResize)
    }
  }, [weatherType, windSpeed, isDark, initParticles])

  if (!weatherType) return null

  return (
    <canvas
      ref={canvasRef}
      className="absolute inset-0"
      style={{ pointerEvents: "none" }}
    />
  )
}

// ────────────────────────────────────────────
// Starry Sky Points (box-shadow approach, dark mode only)
// ────────────────────────────────────────────

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

function ShootingStar({ id, onDone }: { id: number; onDone: (id: number) => void }) {
  const style = useMemo(() => {
    const seeded = (offset: number) => {
      const value = Math.sin((id + 1) * 12.9898 + offset * 78.233) * 43758.5453
      return value - Math.floor(value)
    }
    const top = seeded(1) * 40
    const left = 50 + seeded(2) * 50
    const duration = 0.8 + seeded(3) * 0.6
    const trailWidth = 120 + seeded(4) * 180
    const travelDistance = trailWidth * 2.5
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

// ────────────────────────────────────────────
// Cloud shapes (CSS, used for cloudy/fog)
// ────────────────────────────────────────────

function CloudLayer({ count, isFog }: { count: number; isFog?: boolean }) {
  const clouds = useMemo(() => {
    return Array.from({ length: count }, (_, i) => {
      const seeded = (offset: number) => {
        const v = Math.sin((i + 1) * 9.81 + offset * 41.17) * 28571.3
        return v - Math.floor(v)
      }
      return {
        top: isFog ? 10 + seeded(1) * 80 : seeded(1) * 50,
        width: 180 + seeded(2) * 250,
        height: 50 + seeded(3) * 40,
        opacity: isFog ? 0.12 + seeded(4) * 0.12 : 0.08 + seeded(4) * 0.15,
        duration: 50 + seeded(5) * 60,
        delay: -(seeded(6) * 80),
        startLeft: -20,
      }
    })
  }, [count, isFog])

  return (
    <>
      {clouds.map((c, i) => (
        <div
          key={i}
          className="weather-cloud-shape"
          style={{
            top: `${c.top}%`,
            width: `${c.width}px`,
            height: `${c.height}px`,
            opacity: c.opacity,
            animationDuration: `${c.duration}s`,
            animationDelay: `${c.delay}s`,
          }}
        />
      ))}
    </>
  )
}

// ────────────────────────────────────────────
// Wind Streaks (CSS, visible when windSpeed > 30)
// ────────────────────────────────────────────

function WindStreaks() {
  const streaks = useMemo(() => {
    return Array.from({ length: 8 }, (_, i) => {
      const seeded = (offset: number) => {
        const v = Math.sin((i + 1) * 7.53 + offset * 31.97) * 19937.1
        return v - Math.floor(v)
      }
      return {
        top: 5 + seeded(1) * 85,
        width: 80 + seeded(2) * 200,
        duration: 1.5 + seeded(3) * 2,
        delay: -(seeded(4) * 3),
        opacity: 0.06 + seeded(5) * 0.1,
      }
    })
  }, [])

  return (
    <>
      {streaks.map((s, i) => (
        <div
          key={i}
          className="weather-wind-streak"
          style={{
            top: `${s.top}%`,
            width: `${s.width}px`,
            opacity: s.opacity,
            animationDuration: `${s.duration}s`,
            animationDelay: `${s.delay}s`,
          }}
        />
      ))}
    </>
  )
}

// ────────────────────────────────────────────
// Main Component
// ────────────────────────────────────────────

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
        const effects = await invoke<boolean>("get_ui_effects_enabled")
        if (mounted) setUiEffectsEnabled(effects)
        if (effects) {
          try {
            const w = await invoke<WeatherData | null>("get_current_weather")
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

    let unlistenWeather: (() => void) | null = null
    listen<WeatherData>("weather-cache-updated", (event) => {
      applyWeather(event.payload)
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
