//! trusty-metro-north — MTA Metro North GTFS-RT integration for trusty-izzie.

pub mod client;
pub mod parser;
pub mod stations;

use anyhow::{Context, Result};
use serde_json::Value;

/// Fetch upcoming trains between two stations.
/// Arguments: { "from_station": "...", "to_station": "...", "count": 5 }
pub async fn get_train_schedule(args: &Value) -> Result<String> {
    let api_key = std::env::var("MTA_API_KEY")
        .context("MTA_API_KEY not set. Register at https://api.mta.info to get a free API key.")?;

    let from_name = args["from_station"]
        .as_str()
        .context("Missing from_station")?;
    let to_name = args["to_station"].as_str().context("Missing to_station")?;
    let count = args["count"].as_u64().unwrap_or(5).min(20) as usize;

    let from_id = stations::find_stop_id(from_name)
        .with_context(|| format!("Unknown station: '{from_name}'"))?;
    let to_id =
        stations::find_stop_id(to_name).with_context(|| format!("Unknown station: '{to_name}'"))?;

    let feed = client::fetch_feed(&api_key).await?;
    let departures = parser::extract_departures(&feed, from_id, to_id, count);

    if departures.is_empty() {
        return Ok(format!(
            "No upcoming trains found from {from_name} to {to_name}."
        ));
    }

    use chrono::Local;
    let mut lines = vec![format!(
        "Upcoming trains from {} to {}:",
        from_name, to_name
    )];

    for (i, dep) in departures.iter().enumerate() {
        let local_dep = dep.departure_time.with_timezone(&Local);
        let time_str = local_dep.format("%I:%M %p").to_string();

        let delay_str = match dep.delay_seconds {
            Some(d) if d > 60 => format!(" (+{}min delay)", d / 60),
            Some(d) if d > 0 => " (slight delay)".to_string(),
            _ => String::new(),
        };

        let arrival_str = if let Some(arr) = dep.arrival_time {
            let local_arr = arr.with_timezone(&Local);
            format!(" -> arrives {}", local_arr.format("%I:%M %p"))
        } else {
            String::new()
        };

        let headsign = dep
            .headsign
            .as_deref()
            .map(|h| format!(" to {h}"))
            .unwrap_or_default();

        let trip_short = &dep.trip_id[..dep.trip_id.len().min(8)];
        lines.push(format!(
            "{}. {}{}{}{} [Trip {}]",
            i + 1,
            time_str,
            delay_str,
            arrival_str,
            headsign,
            trip_short
        ));
    }

    Ok(lines.join("\n"))
}

/// Fetch active service alerts.
/// Arguments: { "line": "New Haven" }
pub async fn get_train_alerts(args: &Value) -> Result<String> {
    let api_key = std::env::var("MTA_API_KEY")
        .context("MTA_API_KEY not set. Register at https://api.mta.info to get a free API key.")?;

    let line_filter = args["line"].as_str();

    let feed = client::fetch_feed(&api_key).await?;
    let alerts = parser::extract_alerts(&feed, line_filter);

    if alerts.is_empty() {
        let scope = line_filter
            .map(|l| format!(" on the {l} line"))
            .unwrap_or_default();
        return Ok(format!("No active service alerts{scope}."));
    }

    let mut lines = vec![format!("Active Metro North alerts ({}):", alerts.len())];
    for (i, alert) in alerts.iter().enumerate() {
        lines.push(format!("\n{}. {}", i + 1, alert.header));
        if let Some(desc) = &alert.description {
            let short = desc.chars().take(300).collect::<String>();
            lines.push(format!("   {short}"));
        }
        if !alert.affected_routes.is_empty() {
            lines.push(format!("   Affected: {}", alert.affected_routes.join(", ")));
        }
    }

    Ok(lines.join("\n"))
}
