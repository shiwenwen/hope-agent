export interface WeatherData {
  city: string
  weatherCode: number
  temperature: number
  windSpeed: number
  humidity: number
  weatherDescription: string
  locationName: string
}

export type WeatherType = "cloudy" | "fog" | "rain" | "snow" | "thunder" | null

export function classifyWeather(code: number): WeatherType {
  if (code >= 95) return "thunder"
  if ((code >= 51 && code <= 67) || (code >= 80 && code <= 82)) return "rain"
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "snow"
  if (code === 45 || code === 48) return "fog"
  if (code === 2 || code === 3) return "cloudy"
  if (code <= 1) return null
  return null
}

export function generatePoints(count: number, maxX: number, maxY: number, seed = 1): string {
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
