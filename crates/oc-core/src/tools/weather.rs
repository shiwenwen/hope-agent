use anyhow::Result;
use serde_json::Value;

use crate::weather;

/// Agent tool: get_weather
/// Fetches current weather and optional forecast for a location.
pub(crate) async fn tool_get_weather(args: &Value) -> Result<String> {
    // Determine location: from args or from user config
    let (lat, lon, city) = resolve_location(args).await?;
    let forecast_days = args
        .get("forecast_days")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32)
        .unwrap_or(1)
        .clamp(1, 16);

    let resp = weather::fetch_weather(lat, lon, &city, forecast_days).await?;

    // Format response as structured JSON
    let result = serde_json::to_string_pretty(&resp)?;
    Ok(result)
}

/// Resolve location from tool arguments or user config.
/// Priority: explicit args > user config
async fn resolve_location(args: &Value) -> Result<(f64, f64, String)> {
    // Check for explicit location parameter
    if let Some(location) = args.get("location").and_then(|v| v.as_str()) {
        let location = location.trim();
        if !location.is_empty() {
            // Try parsing as "lat,lon" format
            if let Some((lat, lon)) = parse_lat_lon(location) {
                return Ok((lat, lon, format!("{:.2},{:.2}", lat, lon)));
            }
            // Otherwise treat as city name — geocode it
            let results = weather::geocode_search(location, "en").await?;
            if let Some(first) = results.first() {
                return Ok((first.latitude, first.longitude, first.name.clone()));
            }
            return Err(anyhow::anyhow!(
                "Could not find location: '{}'. Try specifying latitude,longitude instead.",
                location
            ));
        }
    }

    // Fall back to user config
    let cfg = crate::user_config::load_user_config()?;
    match (cfg.weather_latitude, cfg.weather_longitude) {
        (Some(lat), Some(lon)) => {
            let city = cfg.weather_city.unwrap_or_else(|| "Unknown".to_string());
            Ok((lat, lon, city))
        }
        _ => Err(anyhow::anyhow!(
            "No location configured. Either pass a 'location' parameter \
             (city name or 'latitude,longitude') or configure your location \
             in Settings > Profile > Weather."
        )),
    }
}

/// Try to parse a "lat,lon" string.
fn parse_lat_lon(s: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() == 2 {
        let lat = parts[0].trim().parse::<f64>().ok()?;
        let lon = parts[1].trim().parse::<f64>().ok()?;
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
            return Some((lat, lon));
        }
    }
    None
}
