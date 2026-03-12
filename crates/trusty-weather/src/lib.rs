//! trusty-weather -- Open-Meteo forecast + NWS alerts for trusty-izzie.

pub mod alerts;
pub mod forecast;
pub mod geocode;

use anyhow::Result;
use serde_json::Value;

use geocode::{DEFAULT_LAT, DEFAULT_LOCATION, DEFAULT_LON};

/// Resolve location from args or fall back to default.
async fn resolve_location(args: &Value) -> geocode::GeoLocation {
    let loc_str = args["location"].as_str().unwrap_or("");
    geocode::geocode(loc_str)
        .await
        .unwrap_or(geocode::GeoLocation {
            latitude: DEFAULT_LAT,
            longitude: DEFAULT_LON,
            display_name: DEFAULT_LOCATION.to_string(),
        })
}

/// get_weather: { "location": "...", "days": 3 }
pub async fn get_weather(args: &Value) -> Result<String> {
    let loc = resolve_location(args).await;
    let days = args["days"].as_u64().unwrap_or(3) as u32;
    forecast::get_forecast(&loc, days).await
}

/// get_weather_alerts: { "location": "..." }
pub async fn get_weather_alerts(args: &Value) -> Result<String> {
    let loc = resolve_location(args).await;
    alerts::get_alerts(&loc).await
}

/// Used by proactive daemon check -- returns Some(summary) if action needed.
pub async fn proactive_check(lat: f64, lon: f64, location_name: &str) -> Option<String> {
    let loc = geocode::GeoLocation {
        latitude: lat,
        longitude: lon,
        display_name: location_name.to_string(),
    };

    let mut alerts_msg = None;

    // Check NWS alerts
    if let Ok(Some(alert_summary)) = alerts::check_active_alerts(&loc).await {
        alerts_msg = Some(format!("ALERT: {alert_summary}"));
    }

    // Check severe forecast
    let forecast_warning = forecast::check_severe_forecast(&loc).await.ok().flatten();

    match (alerts_msg, forecast_warning) {
        (Some(a), Some(f)) => Some(format!("{a}\nForecast: {f}")),
        (Some(a), None) => Some(a),
        (None, Some(f)) => Some(format!("Weather heads-up: {f}")),
        (None, None) => None,
    }
}
