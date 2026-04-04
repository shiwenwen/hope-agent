import { useMemo } from "react"

export default function CloudLayer({ count, isFog }: { count: number; isFog?: boolean }) {
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
