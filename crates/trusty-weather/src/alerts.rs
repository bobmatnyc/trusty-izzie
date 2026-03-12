//! NWS active weather alerts for a location.

use anyhow::Result;
use serde::Deserialize;

use crate::geocode::GeoLocation;

#[derive(Debug, Deserialize)]
struct AlertsResponse {
    features: Vec<AlertFeature>,
}

#[derive(Debug, Deserialize)]
struct AlertFeature {
    properties: AlertProperties,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlertProperties {
    event: String,
    headline: Option<String>,
    severity: String,
    urgency: String,
    description: Option<String>,
    ends: Option<String>,
}

pub async fn get_alerts(loc: &GeoLocation) -> Result<String> {
    let url = format!(
        "https://api.weather.gov/alerts/active?point={:.4},{:.4}",
        loc.latitude, loc.longitude
    );

    let resp: AlertsResponse = reqwest::Client::builder()
        .user_agent("TrustyIzzie/1.0 (personal assistant)")
        .build()?
        .get(&url)
        .send()
        .await?
        .json()
        .await
        .unwrap_or(AlertsResponse { features: vec![] });

    if resp.features.is_empty() {
        return Ok(format!(
            "No active weather alerts for {}.",
            loc.display_name
        ));
    }

    let mut lines = vec![format!(
        "{} active weather alert{} for {}:\n",
        resp.features.len(),
        if resp.features.len() == 1 { "" } else { "s" },
        loc.display_name
    )];

    for alert in &resp.features {
        let p = &alert.properties;
        let headline = p.headline.as_deref().unwrap_or(&p.event);
        lines.push(headline.to_string());
        lines.push(format!(
            "   Severity: {} | Urgency: {}",
            p.severity, p.urgency
        ));
        if let Some(desc) = &p.description {
            let short: String = desc.chars().take(300).collect();
            lines.push(format!("   {}", short.trim()));
        }
        if let Some(ends) = &p.ends {
            lines.push(format!("   Expires: {ends}"));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

/// Returns active alert summary if any are severe/extreme/moderate, None otherwise.
pub async fn check_active_alerts(loc: &GeoLocation) -> Result<Option<String>> {
    let url = format!(
        "https://api.weather.gov/alerts/active?point={:.4},{:.4}",
        loc.latitude, loc.longitude
    );

    let resp: AlertsResponse = reqwest::Client::builder()
        .user_agent("TrustyIzzie/1.0")
        .build()?
        .get(&url)
        .send()
        .await?
        .json()
        .await
        .unwrap_or(AlertsResponse { features: vec![] });

    let severe: Vec<_> = resp
        .features
        .iter()
        .filter(|f| {
            matches!(
                f.properties.severity.as_str(),
                "Extreme" | "Severe" | "Moderate"
            )
        })
        .collect();

    if severe.is_empty() {
        return Ok(None);
    }

    let events: Vec<&str> = severe.iter().map(|f| f.properties.event.as_str()).collect();

    Ok(Some(format!("Active NWS alerts: {}", events.join(", "))))
}
