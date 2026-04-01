# Weather Geolocation Design

## Overview

Add automatic location detection to the weather region settings. Users can click a single locate button to auto-fill their city and coordinates. The system tries macOS CoreLocation first (precise), then falls back to IP-based geolocation (city-level) with a light toast notification on fallback.

## Architecture

### New Tauri Command

```
detect_location() → Result<GeoLocation, String>
```

**Return type:**

```rust
struct GeoLocation {
    latitude: f64,
    longitude: f64,
    city: Option<String>,
    admin1: Option<String>,
    country: Option<String>,
    source: String, // "system" or "ip"
}
```

### Detection Flow

```
detect_location()
  ├─ Try CoreLocation (macOS only, 5s timeout)
  │   ├─ Success → (lat, lon) → reverse geocode → GeoLocation { source: "system" }
  │   └─ Failure (permission denied / timeout / non-macOS)
  │       └─ Try IP geolocation (ip-api.com)
  │           ├─ Success → GeoLocation { source: "ip" }
  │           └─ Failure → Return error
  └─ (non-macOS) → Skip CoreLocation, go straight to IP
```

### System Location (CoreLocation)

- Use `objc2` + `objc2-core-location` crates for macOS CoreLocation API
- Request `CLLocationManager` with `requestLocation()` (one-shot)
- Accuracy: `kCLLocationAccuracyKilometer` (sufficient for weather)
- Timeout: 5 seconds — if no location delivered, treat as failure
- Handle authorization states: `.notDetermined` triggers request, `.denied`/`.restricted` → immediate fallback

### IP Geolocation

- Endpoint: `http://ip-api.com/json/?fields=status,city,regionName,country,lat,lon`
- Free tier: 45 req/min, no API key needed
- Respects existing proxy settings via `crate::provider::apply_proxy()`
- Parse response: map `lat`/`lon`/`city`/`regionName`/`country` to `GeoLocation`

### Reverse Geocoding

When CoreLocation succeeds, we have coordinates but no city name. Reuse the existing Open-Meteo geocoding API:

- Call `geocode_search()` with a nearby city search, or use Open-Meteo's reverse geocoding endpoint
- Specifically: `https://geocoding-api.open-meteo.com/v1/search?name=&latitude={lat}&longitude={lon}&count=1`
- Actually, Open-Meteo doesn't support reverse geocoding. Instead, use a lightweight reverse geocoding approach:
  - Option A: Use `ip-api.com` as a secondary call just for the city name (even when CoreLocation succeeds)
  - Option B: Use `nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json` (free, no key)
- **Chosen: Option B (Nominatim)** — accurate reverse geocoding, free, and decoupled from IP location

### Reverse Geocode via Nominatim

```
GET https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json&accept-language=zh
```

Response includes `address.city`, `address.state`, `address.country`. Extract city name for display.

## Frontend Changes

### WeatherSection.tsx

**New UI element:** A locate button (LocateFixed icon from lucide-react) placed to the right of the city search input.

**States:**
- Idle: LocateFixed icon, normal color
- Loading: Loader2 icon spinning
- Success: auto-fills city, latitude, longitude fields; triggers weather preview

**Behavior on click:**
1. Set loading state
2. `invoke("detect_location")`
3. On success:
   - Update `weatherCity`, `weatherLatitude`, `weatherLongitude` in config
   - Update search query display
   - If `source === "ip"`, show toast: "已使用网络定位（精度较低）" / "Using network location (lower accuracy)"
4. On error:
   - Show toast with error message
5. Clear loading state

### i18n Keys

```json
{
  "settings.weather.detectLocation": "自动定位 / Auto detect location",
  "settings.weather.locating": "定位中... / Locating...",
  "settings.weather.networkLocationUsed": "已使用网络定位（精度较低） / Using network location (lower accuracy)",
  "settings.weather.locationFailed": "定位失败 / Location detection failed"
}
```

## File Changes

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Add `objc2`, `objc2-core-location`, `objc2-foundation` dependencies (macOS only) |
| `src-tauri/src/weather.rs` | Add `detect_location()`, `ip_geolocate()`, `reverse_geocode()` functions |
| `src-tauri/src/commands/config.rs` | Register `detect_location` Tauri command |
| `src-tauri/src/lib.rs` | Add `detect_location` to `invoke_handler!` |
| `src/components/settings/WeatherSection.tsx` | Add locate button with loading/toast logic |
| `src/i18n/locales/zh.json` | Add location-related translation keys |
| `src/i18n/locales/en.json` | Add location-related translation keys |
| `CHANGELOG.md` | Record new feature |
| `AGENTS.md` | No structural change needed (weather module already documented) |

## Error Handling

- CoreLocation timeout (5s): silent fallback to IP
- CoreLocation permission denied: silent fallback to IP
- IP API failure: return error to frontend, show toast
- Nominatim failure: return coordinates without city name (user can manually search)
- Network unavailable: return error to frontend

## Security

- No sensitive data involved (location is user-initiated, not background tracking)
- IP API called over HTTP (ip-api.com free tier limitation) — acceptable since only returns public IP's approximate location
- Nominatim called over HTTPS
- No API keys required for any service
