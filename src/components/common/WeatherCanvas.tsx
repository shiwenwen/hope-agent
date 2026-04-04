import { useEffect, useRef, useCallback } from "react"
import type { WeatherType } from "./weatherUtils"

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

export default function WeatherCanvas({
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
