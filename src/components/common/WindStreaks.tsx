export default function WindStreaks() {
  const streaks = (() => {
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
  })()

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
