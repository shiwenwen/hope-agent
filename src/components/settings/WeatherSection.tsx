import { useState, useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { MapPin, Search, Cloud, RefreshCw, CircleAlert, Loader2, LocateFixed } from "lucide-react"

interface UserConfig {
  weatherEnabled?: boolean
  weatherCity?: string | null
  weatherLatitude?: number | null
  weatherLongitude?: number | null
  // Plus other fields...
}

interface WeatherSectionProps {
  config: UserConfig
  update: (key: string, value: any) => void
}

interface GeocodeResult {
  name: string
  admin1?: string
  country: string
  latitude: number
  longitude: number
}

interface WeatherData {
  temperature: number
  apparentTemperature: number
  humidity: number
  weatherCode: number
  weatherDescription: string
  windSpeed: number
  locationName: string
  latitude: number
  longitude: number
  time: string
}

export function WeatherSection({ config, update }: WeatherSectionProps) {
  const { t } = useTranslation()

  const [searchQuery, setSearchQuery] = useState(config.weatherCity || "")
  const [searchResults, setSearchResults] = useState<GeocodeResult[]>([])
  const [isSearching, setIsSearching] = useState(false)
  const [showDropdown, setShowDropdown] = useState(false)
  
  const [currentWeather, setCurrentWeather] = useState<WeatherData | null>(null)
  const [isLoadingWeather, setIsLoadingWeather] = useState(false)
  const [weatherError, setWeatherError] = useState(false)
  
  const [isLocating, setIsLocating] = useState(false)
  const [locateMessage, setLocateMessage] = useState<{ text: string; type: "info" | "error" } | null>(null)

  const searchRef = useRef<HTMLDivElement>(null)

  // Sync loaded config city into search query
  useEffect(() => {
    if (config.weatherCity) {
      setSearchQuery(config.weatherCity)
    }
  }, [config.weatherCity])
  
  const weatherEnabled = config.weatherEnabled ?? true

  // Fetch weather when coordinates or enabled state change
  useEffect(() => {
    if (!weatherEnabled) {
      setCurrentWeather(null)
      return
    }
    
    // Only fetch if we have somewhat valid coords
    if (config.weatherLatitude !== undefined && config.weatherLatitude !== null &&
        config.weatherLongitude !== undefined && config.weatherLongitude !== null) {
      const timer = setTimeout(() => {
        fetchWeather()
      }, 500)
      return () => clearTimeout(timer)
    } else {
      // Un-set if we don't have coords
      setCurrentWeather(null)
    }
  }, [config.weatherLatitude, config.weatherLongitude, weatherEnabled])
  
  // Close dropdown on click outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (searchRef.current && !searchRef.current.contains(event.target as Node)) {
        setShowDropdown(false)
      }
    }
    document.addEventListener("mousedown", handleClickOutside)
    return () => document.removeEventListener("mousedown", handleClickOutside)
  }, [])

  // Debounced search
  useEffect(() => {
    const timer = setTimeout(() => {
      if (searchQuery.trim().length > 1 && searchQuery !== config.weatherCity) {
        performSearch(searchQuery.trim())
      } else {
        setSearchResults([])
      }
    }, 500)
    return () => clearTimeout(timer)
  }, [searchQuery, config.weatherCity])

  const performSearch = async (query: string) => {
    setIsSearching(true)
    try {
      const results: GeocodeResult[] = await getTransport().call("geocode_search", { query })
      setSearchResults(results)
      setShowDropdown(true)
    } catch (e) {
      logger.error("api", "geocode_search", "Failed to search city", { error: e })
    } finally {
      setIsSearching(false)
    }
  }

  const fetchWeather = async () => {
    if (config.weatherLatitude == null || config.weatherLongitude == null) return
    setIsLoadingWeather(true)
    setWeatherError(false)
    try {
      const city = config.weatherCity || "Unknown"
      const weather: WeatherData = await getTransport().call("preview_weather", {
        lat: config.weatherLatitude,
        lon: config.weatherLongitude,
        city
      })
      setCurrentWeather(weather)
    } catch (e) {
      logger.error("api", "preview_weather", "Failed to fetch weather", { error: e })
      setWeatherError(true)
    } finally {
      setIsLoadingWeather(false)
    }
  }

  const handleDetectLocation = async () => {
    setIsLocating(true)
    setLocateMessage(null)
    try {
      const result: {
        latitude: number
        longitude: number
        city?: string | null
        admin1?: string | null
        country?: string | null
        source: string
      } = await getTransport().call("detect_location")

      update("weatherLatitude", result.latitude)
      update("weatherLongitude", result.longitude)
      if (result.city) {
        update("weatherCity", result.city)
        setSearchQuery(result.city)
      }

      if (result.source === "ip") {
        setLocateMessage({ text: t("settings.weatherNetworkLocation"), type: "info" })
        setTimeout(() => setLocateMessage(null), 4000)
      }
    } catch (e) {
      logger.error("api", "detect_location", "Failed to detect location", { error: e })
      setLocateMessage({ text: t("settings.weatherLocationFailed"), type: "error" })
      setTimeout(() => setLocateMessage(null), 4000)
    } finally {
      setIsLocating(false)
    }
  }

  const handleSelectCity = (result: GeocodeResult) => {
    setSearchQuery(result.name)
    setShowDropdown(false)
    update("weatherCity", result.name)
    update("weatherLatitude", result.latitude)
    update("weatherLongitude", result.longitude)
    // The useEffect will automatically pick up the new coordinates and fetch the preview
  }

  return (
    <div>
      <div className="text-xs font-medium text-muted-foreground mb-4 px-1 flex items-center gap-2">
        <MapPin className="w-3.5 h-3.5" />
        {t("settings.weatherSection")}
      </div>
      <div className="space-y-5 px-1">
        {/* Toggle Enable Weather */}
        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <Label htmlFor="weather-enable">{t("settings.weatherEnabled")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.weatherEnabledDesc")}
            </p>
          </div>
          <Switch
            id="weather-enable"
            checked={weatherEnabled}
            onCheckedChange={(v) => update("weatherEnabled", v)}
          />
        </div>

        {weatherEnabled && (
          <div className="pl-2 space-y-4 border-l-2 border-border/50">
            {/* City Search */}
            <div className="space-y-1 relative" ref={searchRef}>
              <Label className="text-xs">{t("settings.weatherCity")}</Label>
              <div className="relative flex gap-1.5">
                <div className="relative flex-1">
                  <Input
                    placeholder={t("settings.weatherCityPlaceholder")}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    onFocus={() => {
                      if (searchResults.length > 0) setShowDropdown(true)
                    }}
                    className="pl-9"
                  />
                  <Search className="w-4 h-4 text-muted-foreground absolute left-3 top-2.5" />
                  {isSearching && (
                    <Loader2 className="w-3.5 h-3.5 absolute right-3 top-3 animate-spin text-muted-foreground" />
                  )}
                </div>
                <Button
                  variant="outline"
                  size="icon"
                  className="h-9 w-9 shrink-0"
                  title={t("settings.weatherDetectLocation")}
                  onClick={handleDetectLocation}
                  disabled={isLocating}
                >
                  {isLocating ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <LocateFixed className="w-4 h-4" />
                  )}
                </Button>
              </div>
              
              {/* Dropdown */}
              {showDropdown && searchQuery.trim().length > 1 && (
                <div className="absolute z-10 top-full mt-1 w-full bg-popover border rounded-md shadow-md max-h-48 overflow-y-auto">
                  {searchResults.length === 0 && !isSearching ? (
                    <div className="p-2 text-sm text-muted-foreground text-center">
                      {t("settings.weatherCityNoResults")}
                    </div>
                  ) : (
                    searchResults.map((res, i) => (
                      <div 
                        key={i} 
                        className="px-3 py-2 text-sm hover:bg-accent cursor-pointer flex justify-between items-center"
                        onClick={() => handleSelectCity(res)}
                      >
                        <span>{res.name} <span className="text-muted-foreground text-xs ml-1">{res.admin1 && `${res.admin1}, `}{res.country}</span></span>
                        <span className="text-xs text-muted-foreground">
                          {res.latitude.toFixed(2)}, {res.longitude.toFixed(2)}
                        </span>
                      </div>
                    ))
                  )}
                </div>
              )}
            </div>

            {/* Location detection feedback */}
            {locateMessage && (
              <div className={cn(
                "text-xs px-2 py-1.5 rounded-md",
                locateMessage.type === "info"
                  ? "bg-muted text-muted-foreground"
                  : "bg-destructive/10 text-destructive"
              )}>
                {locateMessage.text}
              </div>
            )}

            {/* Coordinates display (manual overrides) */}
            <div className="grid grid-cols-2 gap-3 pb-2">
              <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">{t("settings.weatherLatitude")}</Label>
                <Input 
                  type="number"
                  step="0.0001"
                  value={config.weatherLatitude ?? ""}
                  onChange={(e) => {
                    const v = parseFloat(e.target.value)
                    update("weatherLatitude", isNaN(v) ? null : v)
                  }}
                  className="h-8 text-sm"
                />
              </div>
              <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">{t("settings.weatherLongitude")}</Label>
                <Input 
                  type="number"
                  step="0.0001"
                  value={config.weatherLongitude ?? ""}
                  onChange={(e) => {
                    const v = parseFloat(e.target.value)
                    update("weatherLongitude", isNaN(v) ? null : v)
                  }}
                  className="h-8 text-sm"
                />
              </div>
            </div>
            
            {/* Weather Preview Box */}
            <div className="mt-2 bg-muted/30 p-3 rounded-md border text-sm flex items-start gap-3">
              <Cloud className="w-5 h-5 text-muted-foreground mt-0.5" />
              <div className="flex-1 min-w-0">
                <div className="font-medium mb-1">{t("settings.weatherPreview")}</div>
                {isLoadingWeather ? (
                  <div className="flex items-center text-muted-foreground text-xs gap-1.5 pt-1">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    Loading...
                  </div>
                ) : weatherError ? (
                  <div className="text-xs text-destructive flex items-center gap-1.5 pt-1">
                    <CircleAlert className="w-3.5 h-3.5" />
                    {t("settings.weatherFetchError")}
                  </div>
                ) : currentWeather ? (
                  <div className="text-xs text-muted-foreground pt-2 pb-1">
                    <div className="grid grid-cols-2 gap-y-2 gap-x-4">
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">🌡️ <span>{t("settings.weatherTemp")}</span></span> <span>{currentWeather.temperature.toFixed(1)}°C</span></div>
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">🧑‍🦱 <span>{t("settings.weatherFeelsLike")}</span></span> <span>{currentWeather.apparentTemperature.toFixed(1)}°C</span></div>
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">☁️ <span>{t("settings.weatherCond")}</span></span> <span>{currentWeather.weatherDescription}</span></div>
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">💧 <span>{t("settings.weatherHumidity")}</span></span> <span>{currentWeather.humidity}%</span></div>
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">💨 <span>{t("settings.weatherWind")}</span></span> <span>{currentWeather.windSpeed.toFixed(1)} km/h</span></div>
                      <div className="flex items-center justify-between gap-1.5"><span className="opacity-80 flex items-center gap-1">⏱️ <span>{t("settings.weatherUpdated")}</span></span> <span>{new Date(currentWeather.time).toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'})}</span></div>
                    </div>
                  </div>
                ) : (
                  <div className="text-xs text-muted-foreground pt-1">
                    {t("settings.weatherNoLocation")}
                  </div>
                )}
              </div>
              <Button 
                variant="ghost" 
                size="icon" 
                className="h-7 w-7 shrink-0" 
                title={t("settings.weatherRefresh")}
                onClick={() => fetchWeather()}
                disabled={isLoadingWeather || config.weatherLatitude == null}
              >
                <RefreshCw className={cn("w-3.5 h-3.5", isLoadingWeather && "animate-spin")} />
              </Button>
            </div>

          </div>
        )}
      </div>
    </div>
  )
}
