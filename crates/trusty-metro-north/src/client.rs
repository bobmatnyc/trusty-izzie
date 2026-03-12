//! HTTP client for MTA Metro North GTFS-Realtime feed.

use anyhow::{Context, Result};
use gtfs_rt::FeedMessage;
use prost::Message;

const MTA_MNR_FEED_URL: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/mnr%2Fgtfs-mnr";

/// Fetch and parse the Metro North GTFS-Realtime feed.
pub async fn fetch_feed(api_key: &str) -> Result<FeedMessage> {
    let client = reqwest::Client::new();
    let bytes = client
        .get(MTA_MNR_FEED_URL)
        .header("x-api-key", api_key)
        .send()
        .await
        .context("Failed to fetch MTA GTFS-RT feed")?
        .error_for_status()
        .context("MTA API returned error status")?
        .bytes()
        .await
        .context("Failed to read MTA response body")?;

    FeedMessage::decode(bytes.as_ref()).context("Failed to decode GTFS-RT protobuf")
}
