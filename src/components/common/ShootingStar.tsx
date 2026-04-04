import { useMemo } from "react"

export default function ShootingStar({ id, onDone }: { id: number; onDone: (id: number) => void }) {
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
