import { useEffect, useMemo, useRef, useState, memo } from "react"

/**
 * StarrySky — subtle starry background for dark mode.
 * Uses pure HTML + CSS for best performance (GPU-composited opacity/transform).
 * Fixed-position overlay with pointer-events:none so it never blocks interaction.
 */

// Generate deterministic star positions via box-shadow strings
function generateStars(count: number, maxX: number, maxY: number, seed = 1): string {
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

function StarrySkyInner() {
  const [isDark, setIsDark] = useState(false)
  const [shootingStars, setShootingStars] = useState<number[]>([])
  const nextId = useRef(0)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Stars are generated once (large virtual canvas, tiled via CSS)
  const [stars] = useState(() => ({
    small: generateStars(200, 2000, 2000, 11),
    medium: generateStars(80, 2000, 2000, 29),
    large: generateStars(30, 2000, 2000, 47),
  }))

  // Watch for .dark class changes on <html>
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

  // Spawn shooting stars periodically
  useEffect(() => {
    if (!isDark || reducedMotion) return

    const cleanupTimers: ReturnType<typeof setTimeout>[] = []

    const scheduleNext = () => {
      const delay = 6000 + Math.random() * 12000 // 6-18s
      timerRef.current = setTimeout(() => {
        const id = nextId.current++
        setShootingStars((prev) => [...prev, id])
        // Fallback: remove after max animation duration (1.4s) + buffer,
        // in case onAnimationEnd doesn't fire (e.g. window hidden by tray)
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
  }, [isDark, reducedMotion])

  const removeShootingStar = (id: number) => {
    setShootingStars((prev) => prev.filter((s) => s !== id))
  }

  if (!isDark) return null

  return (
    <div className="starry-sky-container" aria-hidden="true">
      {/* Three layers of stars, each twinkles at different rate */}
      <div
        className="starry-layer starry-twinkle-1"
        style={{ boxShadow: stars.small, width: 2, height: 2 }}
      />
      <div
        className="starry-layer starry-twinkle-2"
        style={{ boxShadow: stars.medium, width: 3, height: 3 }}
      />
      <div
        className="starry-layer starry-twinkle-3"
        style={{ boxShadow: stars.large, width: 4, height: 4 }}
      />

      {/* Shooting stars */}
      {shootingStars.map((id) => (
        <ShootingStar key={id} id={id} onDone={removeShootingStar} />
      ))}
    </div>
  )
}

const StarrySky = memo(StarrySkyInner)
export default StarrySky
