# Weather Geolocation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "detect location" button to weather settings that auto-fills city/coordinates using macOS CoreLocation (precise) with IP geolocation fallback (city-level).

**Architecture:** New `detect_location` Tauri command orchestrates two location sources: CoreLocation via `objc2` raw FFI on macOS (reads last known location from the app process, triggers permission dialog for "OpenComputer"), and `ip-api.com` HTTP API as cross-platform fallback. Reverse geocoding via Nominatim converts coordinates to city names. Frontend adds a single LocateFixed icon button next to the search input.

**Tech Stack:** Rust (`objc2` FFI, `reqwest`), React (lucide-react icons), Tauri 2 IPC

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/Cargo.toml` | Modify | Add `objc2` + `objc2-foundation` as explicit macOS deps |
| `src-tauri/src/weather.rs` | Modify | Add `detect_location()`, `ip_geolocate()`, `reverse_geocode()`, macOS `system_locate()` |
| `src-tauri/src/commands/config.rs` | Modify | Add `detect_location` Tauri command |
| `src-tauri/src/lib.rs` | Modify | Register `detect_location` in `invoke_handler!` |
| `src-tauri/Info.plist` | Create | `NSLocationWhenInUseUsageDescription` for macOS builds |
| `src/components/settings/WeatherSection.tsx` | Modify | Add locate button with loading/feedback states |
| `src/i18n/locales/zh.json` | Modify | Add 4 location-related i18n keys |
| `src/i18n/locales/en.json` | Modify | Add 4 location-related i18n keys |
| `CHANGELOG.md` | Modify | Record new feature |

---

### Task 1: Backend - Add location detection core logic

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Add `objc2` and `objc2-foundation` as explicit macOS dependencies**

In `src-tauri/Cargo.toml`, add to the existing `[target.'cfg(target_os = "macos")'.dependencies]` section:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2-app-kit = { version = "0.3.2", features = ["NSWindow", "NSColor"] }
objc2 = "0.6"
objc2-foundation = "0.3"
```

- [ ] **Step 2: Add IP geolocation response struct and function to `weather.rs`**

Add after the existing `GeocodingResult` struct (~line 134):

```rust
// ── Location Detection ──────────────────────────────────────────

/// Result of automatic location detection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub city: Option<String>,
    pub admin1: Option<String>,
    pub country: Option<String>,
    /// "system" (CoreLocation) or "ip" (IP geolocation)
    pub source: String,
}

/// ip-api.com response
#[derive(Debug, Deserialize)]
struct IpApiResponse {
    status: String,
    #[serde(default)]
    city: Option<String>,
    #[serde(rename = "regionName", default)]
    region_name: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
}

/// Nominatim reverse geocoding response
#[derive(Debug, Deserialize)]
struct NominatimResponse {
    #[serde(default)]
    address: Option<NominatimAddress>,
}

#[derive(Debug, Deserialize)]
struct NominatimAddress {
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    town: Option<String>,
    #[serde(default)]
    village: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    country: Option<String>,
}
```

- [ ] **Step 3: Implement `ip_geolocate()` function**

Add after the structs:

```rust
/// Detect location via IP geolocation (ip-api.com).
/// Returns city-level accuracy, no permissions needed.
async fn ip_geolocate() -> Result<DetectedLocation> {
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5)),
    )
    .build()?;

    let resp: IpApiResponse = client
        .get("http://ip-api.com/json/?fields=status,city,regionName,country,lat,lon")
        .send()
        .await?
        .json()
        .await?;

    if resp.status != "success" {
        anyhow::bail!("ip-api returned status: {}", resp.status);
    }

    let lat = resp.lat.ok_or_else(|| anyhow::anyhow!("No latitude in IP response"))?;
    let lon = resp.lon.ok_or_else(|| anyhow::anyhow!("No longitude in IP response"))?;

    Ok(DetectedLocation {
        latitude: lat,
        longitude: lon,
        city: resp.city,
        admin1: resp.region_name,
        country: resp.country,
        source: "ip".to_string(),
    })
}
```

- [ ] **Step 4: Implement `reverse_geocode()` function**

```rust
/// Reverse geocode coordinates to a city name via Nominatim.
async fn reverse_geocode(lat: f64, lon: f64) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5)),
    )
    .build()?;

    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&accept-language=zh",
        lat, lon,
    );

    let resp: NominatimResponse = client
        .get(&url)
        .header("User-Agent", "OpenComputer/0.1 (weather-location)")
        .send()
        .await?
        .json()
        .await?;

    if let Some(addr) = resp.address {
        let city = addr.city.or(addr.town).or(addr.village);
        Ok((city, addr.state, addr.country))
    } else {
        Ok((None, None, None))
    }
}
```

- [ ] **Step 5: Implement macOS `system_locate()` function**

```rust
/// Try to get location from macOS CoreLocation.
/// Returns Some((lat, lon)) if location services are authorized and a cached location exists.
/// Returns None if not on macOS, not authorized, or no location available.
#[cfg(target_os = "macos")]
fn system_locate() -> Option<(f64, f64)> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};

    // Link CoreLocation framework
    #[link(name = "CoreLocation", kind = "framework")]
    extern "C" {}

    unsafe {
        let cls = AnyClass::get(c"CLLocationManager")?;

        // Check if location services are enabled globally
        let enabled: bool = msg_send![cls, locationServicesEnabled];
        if !enabled {
            return None;
        }

        // Create manager instance
        let mgr: Retained<AnyObject> = msg_send![cls, new];

        // Check authorization status
        // 0=notDetermined, 1=restricted, 2=denied, 3=authorizedAlways, 4=authorizedWhenInUse
        let status: i32 = msg_send![cls, authorizationStatus];

        match status {
            3 | 4 => {} // authorized — proceed
            0 => {
                // Not determined — request authorization, which shows a dialog
                let _: () = msg_send![&*mgr, requestWhenInUseAuthorization];

                // Poll authorization status for up to 30 seconds
                // (user needs time to read and respond to the system dialog)
                for _ in 0..300 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let new_status: i32 = msg_send![cls, authorizationStatus];
                    if new_status != 0 {
                        if new_status != 3 && new_status != 4 {
                            return None; // user denied
                        }
                        break;
                    }
                }
                let final_status: i32 = msg_send![cls, authorizationStatus];
                if final_status != 3 && final_status != 4 {
                    return None;
                }
            }
            _ => return None, // restricted or denied
        }

        // Read last known location (may be nil if no location ever determined)
        let location: *const AnyObject = msg_send![&*mgr, location];
        if location.is_null() {
            return None;
        }

        // CLLocationCoordinate2D is a C struct { latitude: f64, longitude: f64 }
        #[repr(C)]
        struct CLLocationCoordinate2D {
            latitude: f64,
            longitude: f64,
        }

        let coord: CLLocationCoordinate2D = msg_send![&*location, coordinate];

        // Validate coordinates
        if coord.latitude.is_nan() || coord.longitude.is_nan() {
            return None;
        }
        if coord.latitude.abs() > 90.0 || coord.longitude.abs() > 180.0 {
            return None;
        }

        Some((coord.latitude, coord.longitude))
    }
}

#[cfg(not(target_os = "macos"))]
fn system_locate() -> Option<(f64, f64)> {
    None
}
```

- [ ] **Step 6: Implement public `detect_location()` orchestration function**

```rust
/// Detect user location automatically.
/// Tries macOS CoreLocation first (precise), falls back to IP geolocation (city-level).
/// Reverse geocodes to get a city name.
pub async fn detect_location() -> Result<DetectedLocation> {
    // Step 1: Try system location (macOS CoreLocation)
    let system_result = tokio::task::spawn_blocking(system_locate)
        .await
        .unwrap_or(None);

    if let Some((lat, lon)) = system_result {
        // Got system location — reverse geocode to get city name
        let (city, admin1, country) = reverse_geocode(lat, lon).await.unwrap_or((None, None, None));
        return Ok(DetectedLocation {
            latitude: lat,
            longitude: lon,
            city,
            admin1,
            country,
            source: "system".to_string(),
        });
    }

    // Step 2: Fall back to IP geolocation
    ip_geolocate().await
}
```

- [ ] **Step 7: Verify compilation**

Run: `cd /Users/shiwenwen/Code/OpenComputer && cargo check -p open-computer 2>&1 | head -30`
Expected: Compilation succeeds (or only pre-existing warnings).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/weather.rs
git commit -m "feat(weather): add detect_location with CoreLocation + IP fallback"
```

---

### Task 2: Backend - Register Tauri command

**Files:**
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `detect_location` Tauri command to `commands/config.rs`**

Add after the `refresh_weather` command (~line 521):

```rust
/// Detect user location automatically (CoreLocation → IP fallback).
#[tauri::command]
pub async fn detect_location() -> Result<crate::weather::DetectedLocation, String> {
    crate::weather::detect_location()
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register in `lib.rs` invoke_handler**

In `src-tauri/src/lib.rs`, add `commands::config::detect_location` to the Weather section of the `invoke_handler!` macro (after `commands::config::refresh_weather,` around line 992):

```rust
            // Weather
            commands::config::geocode_search,
            commands::config::preview_weather,
            commands::config::get_current_weather,
            commands::config::refresh_weather,
            commands::config::detect_location,
```

- [ ] **Step 3: Verify compilation**

Run: `cd /Users/shiwenwen/Code/OpenComputer && cargo check -p open-computer 2>&1 | head -30`
Expected: Compilation succeeds.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/config.rs src-tauri/src/lib.rs
git commit -m "feat(weather): register detect_location Tauri command"
```

---

### Task 3: macOS Info.plist for location permission

**Files:**
- Create: `src-tauri/Info.plist`

- [ ] **Step 1: Create Info.plist with location usage description**

Create `src-tauri/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>NSLocationWhenInUseUsageDescription</key>
    <string>OpenComputer needs your location to provide local weather information.</string>
</dict>
</plist>
```

> **Note:** Tauri 2 merges this with the auto-generated Info.plist during production builds. The `NSLocationWhenInUseUsageDescription` is required for macOS to show the "Allow location" dialog. In dev mode (`tauri dev`), CoreLocation may not show the dialog — the IP fallback handles this gracefully.

- [ ] **Step 2: Commit**

```bash
git add src-tauri/Info.plist
git commit -m "feat(weather): add NSLocationWhenInUseUsageDescription for macOS"
```

---

### Task 4: Frontend - Add locate button to WeatherSection

**Files:**
- Modify: `src/components/settings/WeatherSection.tsx`

- [ ] **Step 1: Add LocateFixed import and state variables**

In `WeatherSection.tsx`, update the lucide-react import (line 10):

```tsx
import { MapPin, Search, Cloud, RefreshCw, CircleAlert, Loader2, LocateFixed } from "lucide-react"
```

Add new state variables after the existing state declarations (~line 57):

```tsx
  const [isLocating, setIsLocating] = useState(false)
  const [locateMessage, setLocateMessage] = useState<{ text: string; type: "info" | "error" } | null>(null)
```

- [ ] **Step 2: Add `handleDetectLocation` handler function**

Add after the `handleSelectCity` function (~line 152):

```tsx
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
      } = await invoke("detect_location")

      // Update config with detected location
      update("weatherLatitude", result.latitude)
      update("weatherLongitude", result.longitude)
      if (result.city) {
        update("weatherCity", result.city)
        setSearchQuery(result.city)
      }

      // Show feedback if using IP fallback
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
```

- [ ] **Step 3: Add the locate button to the UI**

Replace the city search `<div className="relative">` block (lines 181-195) to add the locate button next to the search input:

```tsx
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
```

- [ ] **Step 4: Add locate message display**

Add right after the closing `</div>` of the city search container (after the dropdown, before the coordinates grid), around where `{/* Coordinates display */}` comment is:

```tsx
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
```

- [ ] **Step 5: Verify frontend compiles**

Run: `cd /Users/shiwenwen/Code/OpenComputer && npx tsc --noEmit 2>&1 | head -20`
Expected: No new type errors.

- [ ] **Step 6: Commit**

```bash
git add src/components/settings/WeatherSection.tsx
git commit -m "feat(weather): add location detect button to WeatherSection"
```

---

### Task 5: i18n keys

**Files:**
- Modify: `src/i18n/locales/zh.json`
- Modify: `src/i18n/locales/en.json`

- [ ] **Step 1: Add Chinese translations**

In `src/i18n/locales/zh.json`, add after the `"weatherFetchError"` line (~line 701):

```json
    "weatherDetectLocation": "自动定位",
    "weatherNetworkLocation": "已使用网络定位（精度较低）",
    "weatherLocationFailed": "定位失败，请手动搜索城市",
```

- [ ] **Step 2: Add English translations**

In `src/i18n/locales/en.json`, add after the `"weatherFetchError"` line (~line 701):

```json
    "weatherDetectLocation": "Auto detect location",
    "weatherNetworkLocation": "Using network location (lower accuracy)",
    "weatherLocationFailed": "Location detection failed, please search manually",
```

- [ ] **Step 3: Sync remaining languages**

Run: `cd /Users/shiwenwen/Code/OpenComputer && node scripts/sync-i18n.mjs --apply`
Expected: Other locale files get the new keys filled in.

- [ ] **Step 4: Commit**

```bash
git add src/i18n/locales/
git commit -m "feat(weather): add i18n keys for location detection"
```

---

### Task 6: Update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add changelog entry**

Add under the `[Unreleased]` section (or create one if needed):

```markdown
### Added
- Weather settings: auto-detect location button with macOS CoreLocation (precise) and IP geolocation fallback (city-level)
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add weather geolocation to CHANGELOG"
```

---

### Task 7: Integration verification

- [ ] **Step 1: Full backend compilation check**

Run: `cd /Users/shiwenwen/Code/OpenComputer && cargo check -p open-computer 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 2: Frontend type check**

Run: `cd /Users/shiwenwen/Code/OpenComputer && npx tsc --noEmit 2>&1 | tail -5`
Expected: No errors.

- [ ] **Step 3: Lint check**

Run: `cd /Users/shiwenwen/Code/OpenComputer && npm run lint 2>&1 | tail -10`
Expected: No new lint errors.

- [ ] **Step 4: i18n sync check**

Run: `cd /Users/shiwenwen/Code/OpenComputer && node scripts/sync-i18n.mjs --check 2>&1`
Expected: No missing translations.
