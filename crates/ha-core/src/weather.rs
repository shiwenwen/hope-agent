use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tokio::sync::Mutex;

// ── Weather Data Structures ──────────────────────────────────────

/// Current weather data from Open-Meteo API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherData {
    /// Temperature in Celsius
    pub temperature: f64,
    /// Apparent (feels-like) temperature in Celsius
    pub apparent_temperature: f64,
    /// Relative humidity percentage
    pub humidity: f64,
    /// WMO weather code
    pub weather_code: i32,
    /// Weather description (derived from weather_code)
    pub weather_description: String,
    /// Wind speed in km/h
    pub wind_speed: f64,
    /// Location name (from user config)
    pub location_name: String,
    /// Latitude
    pub latitude: f64,
    /// Longitude
    pub longitude: f64,
    /// ISO 8601 timestamp of the observation
    pub time: String,
}

/// Daily forecast data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyForecast {
    pub date: String,
    pub weather_code: i32,
    pub weather_description: String,
    pub temperature_max: f64,
    pub temperature_min: f64,
    pub precipitation_sum: f64,
    pub wind_speed_max: f64,
}

/// Combined weather response for the tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherResponse {
    pub current: WeatherData,
    pub daily: Vec<DailyForecast>,
}

/// Geocoding search result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoResult {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: String,
    #[serde(default)]
    pub admin1: Option<String>,
    /// Country code (e.g. "CN", "US")
    #[serde(default)]
    pub country_code: Option<String>,
    /// Population (for sorting relevance)
    #[serde(default)]
    pub population: Option<u64>,
    /// Elevation in meters
    #[serde(default)]
    pub elevation: Option<f64>,
    /// Timezone identifier
    #[serde(default)]
    pub timezone: Option<String>,
}

// ── Open-Meteo API Response Structures ───────────────────────────

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrentResponse {
    #[allow(dead_code)]
    latitude: f64,
    #[allow(dead_code)]
    longitude: f64,
    current: OpenMeteoCurrent,
    #[serde(default)]
    daily: Option<OpenMeteoDaily>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrent {
    time: String,
    temperature_2m: f64,
    relative_humidity_2m: f64,
    apparent_temperature: f64,
    weather_code: i32,
    wind_speed_10m: f64,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoDaily {
    time: Vec<String>,
    weather_code: Vec<i32>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_sum: Vec<f64>,
    wind_speed_10m_max: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct GeocodingResponse {
    #[serde(default)]
    results: Vec<GeocodingResult>,
}

#[derive(Debug, Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country: String,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    population: Option<u64>,
    #[serde(default)]
    elevation: Option<f64>,
    #[serde(default)]
    timezone: Option<String>,
}

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

// ── Weather Cache ────────────────────────────────────────────────

/// Cached weather data with change detection
struct CachedWeather {
    data: Option<WeatherData>,
    /// Hash of the weather data for change detection
    data_hash: Option<String>,
    /// Last fetch timestamp
    last_fetched: Option<std::time::Instant>,
    /// Prompt text (only updated when weather changes)
    prompt_text: Option<String>,
}

/// Cache refresh interval: 30 minutes
const CACHE_REFRESH_SECS: u64 = 30 * 60;

static WEATHER_CACHE: OnceLock<Mutex<CachedWeather>> = OnceLock::new();

fn get_cache() -> &'static Mutex<CachedWeather> {
    WEATHER_CACHE.get_or_init(|| {
        Mutex::new(CachedWeather {
            data: None,
            data_hash: None,
            last_fetched: None,
            prompt_text: None,
        })
    })
}

/// Compute a simple hash for weather change detection.
/// Only considers temperature (rounded to 1°C) and weather code.
fn compute_weather_hash(data: &WeatherData) -> String {
    format!(
        "t:{:.0}_c:{}_h:{:.0}",
        data.temperature, data.weather_code, data.humidity
    )
}

/// Build the prompt text for system prompt injection.
fn build_prompt_text(data: &WeatherData) -> String {
    format!(
        "# Current Weather\n\n\
         - Location: {} ({:.2}°N, {:.2}°E)\n\
         - Temperature: {:.1}°C (feels like {:.1}°C)\n\
         - Weather: {}\n\
         - Humidity: {:.0}%\n\
         - Wind: {:.1} km/h\n\
         - Updated: {}",
        data.location_name,
        data.latitude,
        data.longitude,
        data.temperature,
        data.apparent_temperature,
        data.weather_description,
        data.humidity,
        data.wind_speed,
        data.time,
    )
}

// ── Public API ───────────────────────────────────────────────────

/// Fetch current weather + optional forecast from Open-Meteo.
pub async fn fetch_weather(
    lat: f64,
    lon: f64,
    location_name: &str,
    forecast_days: u32,
) -> Result<WeatherResponse> {
    let forecast_days = forecast_days.clamp(1, 16);

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={}&longitude={}&\
         current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&\
         daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,wind_speed_10m_max&\
         forecast_days={}&\
         timezone=auto",
        lat, lon, forecast_days,
    );

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(10)),
    )
    .build()?;

    let resp: OpenMeteoCurrentResponse = client.get(&url).send().await?.json().await?;

    let current = WeatherData {
        temperature: resp.current.temperature_2m,
        apparent_temperature: resp.current.apparent_temperature,
        humidity: resp.current.relative_humidity_2m,
        weather_code: resp.current.weather_code,
        weather_description: weather_code_description(resp.current.weather_code).to_string(),
        wind_speed: resp.current.wind_speed_10m,
        location_name: location_name.to_string(),
        latitude: lat,
        longitude: lon,
        time: resp.current.time.clone(),
    };

    let mut daily = Vec::new();
    if let Some(d) = resp.daily {
        for i in 0..d.time.len() {
            daily.push(DailyForecast {
                date: d.time[i].clone(),
                weather_code: d.weather_code[i],
                weather_description: weather_code_description(d.weather_code[i]).to_string(),
                temperature_max: d.temperature_2m_max[i],
                temperature_min: d.temperature_2m_min[i],
                precipitation_sum: d.precipitation_sum[i],
                wind_speed_max: d.wind_speed_10m_max[i],
            });
        }
    }

    Ok(WeatherResponse { current, daily })
}

/// Search for cities by name using Open-Meteo Geocoding API.
pub async fn geocode_search(query: &str, language: &str) -> Result<Vec<GeoResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let lang = if language.starts_with("zh") {
        "zh"
    } else {
        "en"
    };

    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language={}&format=json",
        urlencoding::encode(query.trim()),
        lang,
    );

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5)),
    )
    .build()?;

    let resp: GeocodingResponse = client.get(&url).send().await?.json().await?;

    Ok(resp
        .results
        .into_iter()
        .map(|r| GeoResult {
            name: r.name,
            latitude: r.latitude,
            longitude: r.longitude,
            country: r.country,
            admin1: r.admin1,
            country_code: r.country_code,
            population: r.population,
            elevation: r.elevation,
            timezone: r.timezone,
        })
        .collect())
}

/// Get weather text for system prompt injection.
/// Returns cached prompt text (only updates when weather changes).
/// Returns None if no weather data is available or weather is disabled.
pub fn get_weather_for_prompt() -> Option<String> {
    // Check if weather is enabled in user config
    let user_cfg = crate::user_config::load_user_config().ok()?;
    if !user_cfg.weather_enabled {
        return None;
    }
    // Must have location configured
    if user_cfg.weather_latitude.is_none() || user_cfg.weather_longitude.is_none() {
        return None;
    }

    let cache = get_cache().try_lock().ok()?;
    cache.prompt_text.clone()
}

/// Refresh the weather cache. Called periodically by background task.
/// Only updates prompt text when weather actually changes.
pub async fn refresh_weather_cache() {
    let user_cfg = match crate::user_config::load_user_config() {
        Ok(c) => c,
        Err(_) => return,
    };

    if !user_cfg.weather_enabled {
        return;
    }

    let lat = match user_cfg.weather_latitude {
        Some(l) => l,
        None => return,
    };
    let lon = match user_cfg.weather_longitude {
        Some(l) => l,
        None => return,
    };

    let city = user_cfg.weather_city.as_deref().unwrap_or("Unknown");

    // Check if cache is still fresh
    {
        let cache = get_cache().lock().await;
        if let Some(last) = cache.last_fetched {
            if last.elapsed().as_secs() < CACHE_REFRESH_SECS {
                return;
            }
        }
    }

    // Fetch fresh data
    match fetch_weather(lat, lon, city, 1).await {
        Ok(resp) => {
            let new_hash = compute_weather_hash(&resp.current);
            let mut cache = get_cache().lock().await;

            let changed = cache
                .data_hash
                .as_ref()
                .map(|old| old != &new_hash)
                .unwrap_or(true);

            if changed {
                // Weather changed — update prompt text
                let prompt = build_prompt_text(&resp.current);
                cache.prompt_text = Some(prompt);
                cache.data_hash = Some(new_hash);

                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "info",
                        "weather",
                        "refresh_cache",
                        &format!(
                            "Weather updated: {}°C, {} ({})",
                            resp.current.temperature, resp.current.weather_description, city
                        ),
                        None,
                        None,
                        None,
                    );
                }
            }

            cache.data = Some(resp.current.clone());
            cache.last_fetched = Some(std::time::Instant::now());

            // Notify frontend about weather cache update
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "weather-cache-updated",
                    serde_json::to_value(&resp.current).unwrap_or_default(),
                );
            }
        }
        Err(e) => {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "weather",
                    "refresh_cache",
                    &format!("Failed to fetch weather: {}", e),
                    None,
                    None,
                    None,
                );
            }
        }
    }
}

/// Get the currently cached weather data (for frontend preview).
pub async fn get_cached_weather() -> Option<WeatherData> {
    let cache = get_cache().lock().await;
    cache.data.clone()
}

/// Force refresh weather cache, ignoring the time check.
pub async fn force_refresh_weather() -> Result<Option<WeatherData>> {
    let user_cfg = crate::user_config::load_user_config()?;

    if !user_cfg.weather_enabled {
        return Ok(None);
    }

    let lat = user_cfg
        .weather_latitude
        .ok_or_else(|| anyhow::anyhow!("No latitude configured"))?;
    let lon = user_cfg
        .weather_longitude
        .ok_or_else(|| anyhow::anyhow!("No longitude configured"))?;
    let city = user_cfg.weather_city.as_deref().unwrap_or("Unknown");

    let resp = fetch_weather(lat, lon, city, 1).await?;
    let new_hash = compute_weather_hash(&resp.current);

    let mut cache = get_cache().lock().await;
    let prompt = build_prompt_text(&resp.current);
    cache.prompt_text = Some(prompt);
    cache.data_hash = Some(new_hash);
    cache.data = Some(resp.current.clone());
    cache.last_fetched = Some(std::time::Instant::now());

    // Notify frontend about weather cache update
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "weather-cache-updated",
            serde_json::to_value(&resp.current).unwrap_or_default(),
        );
    }

    Ok(Some(resp.current))
}

/// Start the background weather refresh task.
/// Should be called once during app setup.
pub fn start_background_refresh() {
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create weather runtime");
        rt.block_on(async {
            // Initial fetch on startup (with a small delay to not block startup)
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            refresh_weather_cache().await;

            // Periodic refresh loop
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(CACHE_REFRESH_SECS));
            loop {
                interval.tick().await;
                refresh_weather_cache().await;
            }
        });
    });
}

// ── WMO Weather Code Mapping ─────────────────────────────────────

/// Convert WMO weather code to human-readable description.
/// Reference: https://open-meteo.com/en/docs
pub fn weather_code_description(code: i32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Foggy",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Dense freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Heavy freezing rain",
        71 => "Slight snowfall",
        73 => "Moderate snowfall",
        75 => "Heavy snowfall",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "Thunderstorm with slight hail",
        99 => "Thunderstorm with heavy hail",
        _ => "Unknown",
    }
}

// ── Location Detection Functions ────────────────────────────────

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

    let lat = resp
        .lat
        .ok_or_else(|| anyhow::anyhow!("No latitude in IP response"))?;
    let lon = resp
        .lon
        .ok_or_else(|| anyhow::anyhow!("No longitude in IP response"))?;

    Ok(DetectedLocation {
        latitude: lat,
        longitude: lon,
        city: resp.city,
        admin1: resp.region_name,
        country: resp.country,
        source: "ip".to_string(),
    })
}

/// Reverse geocode coordinates to a city name via Nominatim.
async fn reverse_geocode(
    lat: f64,
    lon: f64,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
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
        .header("User-Agent", "Hope Agent/0.1 (weather-location)")
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

#[cfg(target_os = "macos")]
async fn system_locate() -> Option<(f64, f64)> {
    crate::weather_location_macos::system_locate().await
}

#[cfg(not(target_os = "macos"))]
async fn system_locate() -> Option<(f64, f64)> {
    crate::app_info!(
        "weather",
        "system_locate",
        "Not macOS, skipping CoreLocation"
    );
    None
}

/// Detect user location automatically.
/// Tries macOS CoreLocation first (precise), falls back to IP geolocation (city-level).
/// Reverse geocodes to get a city name when using system location.
pub async fn detect_location() -> Result<DetectedLocation> {
    crate::app_info!("weather", "detect_location", "Starting location detection");

    // Step 1: Try system location (macOS CoreLocation)
    let system_result = system_locate().await;

    if let Some((lat, lon)) = system_result {
        crate::app_info!(
            "weather",
            "detect_location",
            "System location obtained, reverse geocoding lat={:.4}, lon={:.4}",
            lat,
            lon
        );
        let (city, admin1, country) = reverse_geocode(lat, lon)
            .await
            .unwrap_or((None, None, None));
        crate::app_info!(
            "weather",
            "detect_location",
            "Result: source=system, city={:?}",
            city
        );
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
    crate::app_info!(
        "weather",
        "detect_location",
        "System location unavailable, falling back to IP geolocation"
    );
    let result = ip_geolocate().await;
    match &result {
        Ok(loc) => crate::app_info!(
            "weather",
            "detect_location",
            "Result: source=ip, city={:?}, lat={:.4}, lon={:.4}",
            loc.city,
            loc.latitude,
            loc.longitude
        ),
        Err(e) => crate::app_error!(
            "weather",
            "detect_location",
            "IP geolocation also failed: {}",
            e
        ),
    }
    result
}
