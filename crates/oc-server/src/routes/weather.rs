use axum::extract::Query;
use axum::Json;
use serde::Deserialize;

use oc_core::weather;

use crate::error::AppError;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub language: Option<String>,
}

/// `GET /api/weather/geocode?query=...&language=...`
pub async fn geocode_search(
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<weather::GeoResult>>, AppError> {
    let lang = q.language.as_deref().unwrap_or("zh");
    Ok(Json(weather::geocode_search(&q.query, lang).await?))
}

#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    pub lat: f64,
    pub lon: f64,
    pub city: String,
}

/// `POST /api/weather/preview`
pub async fn preview_weather(
    Json(q): Json<PreviewQuery>,
) -> Result<Json<weather::WeatherData>, AppError> {
    let out = weather::fetch_weather(q.lat, q.lon, &q.city, 1).await?;
    Ok(Json(out.current))
}

/// `GET /api/weather/current`
pub async fn get_current_weather() -> Result<Json<Option<weather::WeatherData>>, AppError> {
    Ok(Json(weather::get_cached_weather().await))
}

/// `POST /api/weather/refresh`
pub async fn refresh_weather() -> Result<Json<Option<weather::WeatherData>>, AppError> {
    Ok(Json(weather::force_refresh_weather().await?))
}

/// `GET /api/weather/detect-location`
pub async fn detect_location() -> Result<Json<weather::DetectedLocation>, AppError> {
    Ok(Json(weather::detect_location().await?))
}
