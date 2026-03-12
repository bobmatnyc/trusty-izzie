//! Open-Meteo forecast fetching and formatting.

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use serde::Deserialize;

use crate::geocode::GeoLocation;

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    hourly: HourlyData,
    daily: DailyData,
}

#[derive(Debug, Deserialize)]
struct HourlyData {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    precipitation_probability: Vec<Option<f64>>,
    weathercode: Vec<Option<u32>>,
    windspeed_10m: Vec<Option<f64>>,
    #[allow(dead_code)]
    windgusts_10m: Vec<Option<f64>>,
}

#[derive(Debug, Deserialize)]
struct DailyData {
    time: Vec<String>,
    weathercode: Vec<Option<u32>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<f64>>,
    precipitation_sum: Vec<Option<f64>>,
    windspeed_10m_max: Vec<Option<f64>>,
}

pub fn wmo_description(code: u32) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
        51 | 53 | 55 => "Drizzle",
        61 => "Light rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        71 => "Light snow",
        73 => "Moderate snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

pub async fn get_forecast(loc: &GeoLocation, days: u32) -> Result<String> {
    let days = days.clamp(1, 7);
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
        ?latitude={lat}&longitude={lon}\
        &hourly=temperature_2m,precipitation_probability,weathercode,windspeed_10m,windgusts_10m\
        &daily=weathercode,temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum,windspeed_10m_max\
        &temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch\
        &timezone=America%2FNew_York&forecast_days={days}",
        lat = loc.latitude,
        lon = loc.longitude,
        days = days
    );

    let resp: ForecastResponse = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .context("Weather request failed")?
        .error_for_status()
        .context("Weather API error")?
        .json()
        .await
        .context("Failed to parse weather response")?;

    let today = Local::now().date_naive();
    let mut lines = vec![format!(
        "Weather for {} ({} day{})\n",
        loc.display_name,
        days,
        if days == 1 { "" } else { "s" }
    )];

    for (i, date_str) in resp.daily.time.iter().enumerate() {
        let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };
        let label = if date == today {
            "Today".to_string()
        } else if date == today + chrono::Duration::days(1) {
            "Tomorrow".to_string()
        } else {
            date.format("%A, %b %d").to_string()
        };

        let code = resp.daily.weathercode.get(i).and_then(|v| *v).unwrap_or(0);
        let hi = resp
            .daily
            .temperature_2m_max
            .get(i)
            .and_then(|v| *v)
            .map(|t| format!("{t:.0}F"))
            .unwrap_or_default();
        let lo = resp
            .daily
            .temperature_2m_min
            .get(i)
            .and_then(|v| *v)
            .map(|t| format!("{t:.0}F"))
            .unwrap_or_default();
        let pop = resp
            .daily
            .precipitation_probability_max
            .get(i)
            .and_then(|v| *v)
            .map(|p| format!("{p:.0}% precip"))
            .unwrap_or_default();
        let precip = resp
            .daily
            .precipitation_sum
            .get(i)
            .and_then(|v| *v)
            .filter(|&p| p > 0.01)
            .map(|p| format!(", {p:.2}\""))
            .unwrap_or_default();
        let wind = resp
            .daily
            .windspeed_10m_max
            .get(i)
            .and_then(|v| *v)
            .map(|w| format!("{w:.0}mph winds"))
            .unwrap_or_default();

        let desc = wmo_description(code);
        lines.push(format!(
            "{label}: {desc} | {hi}/{lo} | {pop}{precip} | {wind}"
        ));
    }

    // Add hourly detail for today (next 6 hours from now)
    let now_hour = Local::now().format("%H:00").to_string();
    let today_str = today.format("%Y-%m-%d").to_string();
    let mut hourly_lines = vec![];
    let mut count = 0;
    for (i, time) in resp.hourly.time.iter().enumerate() {
        if !time.starts_with(&today_str) {
            continue;
        }
        let hour_part = &time[11..]; // "HH:00"
        if hour_part < now_hour.as_str() {
            continue;
        }
        if count >= 6 {
            break;
        }
        let temp = resp
            .hourly
            .temperature_2m
            .get(i)
            .and_then(|v| *v)
            .map(|t| format!("{t:.0}F"))
            .unwrap_or_default();
        let pop = resp
            .hourly
            .precipitation_probability
            .get(i)
            .and_then(|v| *v)
            .map(|p| format!("{p:.0}% rain"))
            .unwrap_or_default();
        let code = resp.hourly.weathercode.get(i).and_then(|v| *v).unwrap_or(0);
        let wind = resp
            .hourly
            .windspeed_10m
            .get(i)
            .and_then(|v| *v)
            .map(|w| format!("{w:.0}mph"))
            .unwrap_or_default();
        let hour_display = if let Ok(h) = hour_part[..2].parse::<u32>() {
            if h == 0 {
                "12am".to_string()
            } else if h < 12 {
                format!("{h}am")
            } else if h == 12 {
                "12pm".to_string()
            } else {
                format!("{}pm", h - 12)
            }
        } else {
            hour_part.to_string()
        };
        let _ = code; // used implicitly via wmo_description if needed
        hourly_lines.push(format!("  {hour_display}: {temp} {pop} {wind}"));
        count += 1;
    }
    if !hourly_lines.is_empty() {
        lines.push("\nHourly today:".to_string());
        lines.extend(hourly_lines);
    }

    Ok(lines.join("\n"))
}

/// Quick check: is severe weather forecast in the next 2 days?
/// Returns a description if yes, None if conditions are benign.
pub async fn check_severe_forecast(loc: &GeoLocation) -> Result<Option<String>> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
        ?latitude={lat}&longitude={lon}\
        &daily=weathercode,temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum,windspeed_10m_max\
        &temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch\
        &timezone=America%2FNew_York&forecast_days=2",
        lat = loc.latitude,
        lon = loc.longitude,
    );

    let resp: serde_json::Value = reqwest::Client::new()
        .get(&url)
        .send()
        .await?
        .json()
        .await?;

    let daily = &resp["daily"];
    let mut warnings = vec![];

    for i in 0..2usize {
        let day_label = if i == 0 { "Today" } else { "Tomorrow" };
        let code = daily["weathercode"][i].as_u64().unwrap_or(0) as u32;
        let pop = daily["precipitation_probability_max"][i]
            .as_f64()
            .unwrap_or(0.0);
        let precip = daily["precipitation_sum"][i].as_f64().unwrap_or(0.0);
        let hi = daily["temperature_2m_max"][i].as_f64().unwrap_or(70.0);
        let lo = daily["temperature_2m_min"][i].as_f64().unwrap_or(40.0);
        let wind = daily["windspeed_10m_max"][i].as_f64().unwrap_or(0.0);

        if matches!(code, 65 | 75 | 82 | 86 | 95 | 96 | 99) {
            warnings.push(format!("{day_label}: {} expected", wmo_description(code)));
        }
        if pop >= 80.0 && precip >= 0.5 {
            warnings.push(format!(
                "{day_label}: Heavy rain likely ({precip:.1}\" / {pop:.0}% chance)"
            ));
        }
        if hi >= 95.0 {
            warnings.push(format!("{day_label}: Extreme heat ({hi:.0}F high)"));
        }
        if lo <= 15.0 {
            warnings.push(format!(
                "{day_label}: Dangerously cold overnight ({lo:.0}F low)"
            ));
        }
        if wind >= 35.0 {
            warnings.push(format!("{day_label}: High winds ({wind:.0}mph)"));
        }
    }

    if warnings.is_empty() {
        Ok(None)
    } else {
        Ok(Some(warnings.join("; ")))
    }
}
