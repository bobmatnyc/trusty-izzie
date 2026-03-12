//! Location name -> (lat, lon, display_name) via Open-Meteo geocoding.

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub display_name: String,
}

/// Default location: Hastings-on-Hudson, NY (user's home)
pub const DEFAULT_LAT: f64 = 40.9887;
pub const DEFAULT_LON: f64 = -73.8827;
pub const DEFAULT_LOCATION: &str = "Hastings-on-Hudson, NY";

pub async fn geocode(location: &str) -> Result<GeoLocation> {
    // Handle empty/default case
    if location.is_empty()
        || location.to_lowercase().contains("hastings")
        || location.to_lowercase().contains("home")
    {
        return Ok(GeoLocation {
            latitude: DEFAULT_LAT,
            longitude: DEFAULT_LON,
            display_name: DEFAULT_LOCATION.to_string(),
        });
    }

    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        urlencoding::encode(location)
    );

    let resp: serde_json::Value = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .context("Geocoding request failed")?
        .json()
        .await
        .context("Failed to parse geocoding response")?;

    let result = resp["results"]
        .as_array()
        .and_then(|r| r.first())
        .context("Location not found")?;

    let lat = result["latitude"].as_f64().context("No latitude")?;
    let lon = result["longitude"].as_f64().context("No longitude")?;
    let name = result["name"].as_str().unwrap_or(location);
    let admin = result["admin1"].as_str().unwrap_or("");
    let country = result["country_code"].as_str().unwrap_or("");
    let display = if admin.is_empty() {
        format!("{name}, {country}")
    } else {
        format!("{name}, {admin}")
    };

    Ok(GeoLocation {
        latitude: lat,
        longitude: lon,
        display_name: display,
    })
}
