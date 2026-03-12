//! Parse GTFS-RT FeedMessage into friendly train schedule structs.

use chrono::{DateTime, TimeZone, Utc};
use gtfs_rt::FeedMessage;
use serde::{Deserialize, Serialize};

use crate::stations::find_station_name;

#[derive(Debug, Serialize, Deserialize)]
pub struct TrainDeparture {
    pub trip_id: String,
    pub headsign: Option<String>,
    pub departure_time: DateTime<Utc>,
    pub arrival_time: Option<DateTime<Utc>>,
    pub track: Option<String>,
    pub delay_seconds: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceAlert {
    pub header: String,
    pub description: Option<String>,
    pub affected_routes: Vec<String>,
}

/// Extract upcoming departures from `from_stop_id` to `to_stop_id`.
pub fn extract_departures(
    feed: &FeedMessage,
    from_stop_id: u32,
    to_stop_id: u32,
    count: usize,
) -> Vec<TrainDeparture> {
    let now = Utc::now().timestamp(); // i64
    let from_str = from_stop_id.to_string();
    let to_str = to_stop_id.to_string();

    let mut departures: Vec<TrainDeparture> = feed
        .entity
        .iter()
        .filter_map(|entity| entity.trip_update.as_ref())
        .filter_map(|tu| {
            let stop_times = &tu.stop_time_update;

            // Find the position of from_stop and to_stop in this trip
            let from_idx = stop_times
                .iter()
                .position(|s| s.stop_id.as_deref() == Some(&from_str))?;
            let to_idx = stop_times
                .iter()
                .position(|s| s.stop_id.as_deref() == Some(&to_str))?;

            // to_stop must come after from_stop
            if to_idx <= from_idx {
                return None;
            }

            let from_stop = &stop_times[from_idx];
            let to_stop = &stop_times[to_idx];

            let departure_ts = from_stop
                .departure
                .as_ref()
                .and_then(|d| d.time)
                .or_else(|| from_stop.arrival.as_ref().and_then(|a| a.time))?;

            // Only show future departures
            if departure_ts < now {
                return None;
            }

            let departure_time = Utc.timestamp_opt(departure_ts, 0).single()?;

            let arrival_time = to_stop
                .arrival
                .as_ref()
                .and_then(|a| a.time)
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single());

            let delay_seconds = from_stop
                .departure
                .as_ref()
                .and_then(|d| d.delay)
                .or_else(|| from_stop.arrival.as_ref().and_then(|a| a.delay));

            // trip is a required field so it's TripDescriptor directly (not Option)
            let trip_id = tu.trip.trip_id.clone().unwrap_or_default();

            let headsign = stop_times
                .last()
                .and_then(|s| s.stop_id.as_ref())
                .and_then(|id| id.parse::<u32>().ok())
                .and_then(find_station_name)
                .map(|s| s.to_string())
                .or_else(|| tu.trip.route_id.clone());

            Some(TrainDeparture {
                trip_id,
                headsign,
                departure_time,
                arrival_time,
                track: None,
                delay_seconds,
            })
        })
        .collect();

    departures.sort_by_key(|d| d.departure_time);
    departures.truncate(count);
    departures
}

/// Extract active service alerts.
pub fn extract_alerts(feed: &FeedMessage, line_filter: Option<&str>) -> Vec<ServiceAlert> {
    feed.entity
        .iter()
        .filter_map(|entity| entity.alert.as_ref())
        .filter_map(|alert| {
            // text is a required String field in Translation, not Option<String>
            let header = alert
                .header_text
                .as_ref()
                .and_then(|t| t.translation.first())
                .map(|tr| tr.text.clone())?;

            let description = alert
                .description_text
                .as_ref()
                .and_then(|t| t.translation.first())
                .map(|tr| tr.text.clone());

            let affected_routes: Vec<String> = alert
                .informed_entity
                .iter()
                .filter_map(|e| e.route_id.clone())
                .collect();

            // Apply line filter if specified
            if let Some(filter) = line_filter {
                let filter_lower = filter.to_lowercase();
                let matches = affected_routes
                    .iter()
                    .any(|r| r.to_lowercase().contains(&filter_lower));
                if !affected_routes.is_empty() && !matches {
                    return None;
                }
            }

            Some(ServiceAlert {
                header,
                description,
                affected_routes,
            })
        })
        .collect()
}
