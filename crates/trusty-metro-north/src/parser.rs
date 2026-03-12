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

            // Check stop_time_properties.assigned_stop_id first (GTFS platform assignment).
            // MTA populates this ~20-30 min before departure at GCT; outside that window
            // both fields are absent and track stays None (confirmed by live feed inspection).
            // Fall back to parsing a track suffix from stop_id itself (e.g. "1T03").
            let assigned = from_stop
                .stop_time_properties
                .as_ref()
                .and_then(|p| p.assigned_stop_id.as_deref());
            let raw_stop = from_stop.stop_id.as_deref();
            let track = extract_track_from_stop_id(assigned.or(raw_stop));

            Some(TrainDeparture {
                trip_id,
                headsign,
                departure_time,
                arrival_time,
                track,
                delay_seconds,
            })
        })
        .collect();

    departures.sort_by_key(|d| d.departure_time);
    departures.truncate(count);
    departures
}

/// Extract track number from a GTFS stop_id when it encodes platform/track info.
///
/// MTA Metro North uses `StopTimeProperties.assigned_stop_id` for real-time track
/// assignments at Grand Central Terminal.  When populated, those IDs follow the
/// pattern `<station_digits>T<track_digits>` (e.g. "1T03" = GCT track 3, or
/// "001T003" with zero-padded variants).  The same suffix convention may appear
/// directly on `stop_id` in some feed versions.
///
/// Returns `None` when no track suffix is found so the field stays optional.
/// Leading zeros are stripped (e.g. "03" → "3").
fn extract_track_from_stop_id(stop_id: Option<&str>) -> Option<String> {
    let id = stop_id?;
    // Locate the 'T' separator between station digits and track digits.
    // Only treat it as a track encoding when both sides are purely numeric.
    let upper = id.to_uppercase();
    let t_pos = upper.find('T')?;
    let station_part = &upper[..t_pos];
    let track_part = &id[t_pos + 1..]; // preserve original casing for digits
    if station_part.is_empty()
        || !station_part.chars().all(|c| c.is_ascii_digit())
        || track_part.is_empty()
        || !track_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // Strip leading zeros by parsing as u32 then converting back.
    let track_num: u32 = track_part.parse().ok()?;
    Some(track_num.to_string())
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

#[cfg(test)]
mod tests {
    use super::extract_track_from_stop_id;

    #[test]
    fn test_extract_track_basic() {
        assert_eq!(extract_track_from_stop_id(Some("1T3")), Some("3".into()));
        assert_eq!(extract_track_from_stop_id(Some("1T03")), Some("3".into()));
        assert_eq!(
            extract_track_from_stop_id(Some("001T003")),
            Some("3".into())
        );
        assert_eq!(extract_track_from_stop_id(Some("1T42")), Some("42".into()));
    }

    #[test]
    fn test_extract_track_no_match() {
        // Plain numeric stop_id with no T separator
        assert_eq!(extract_track_from_stop_id(Some("1")), None);
        // T present but non-numeric sides
        assert_eq!(extract_track_from_stop_id(Some("GCT_T1")), None);
        assert_eq!(extract_track_from_stop_id(Some("1TX")), None);
        // None input
        assert_eq!(extract_track_from_stop_id(None), None);
    }

    /// Live feed diagnostic — run manually to inspect what the feed actually contains.
    /// Prints stop_id and assigned_stop_id for the first 20 StopTimeUpdates at GCT (stop 1).
    /// Run with: cargo test -p trusty-metro-north -- --ignored --nocapture diagnose_live_feed_track_fields
    #[test]
    #[ignore]
    fn diagnose_live_feed_track_fields() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let feed = rt
            .block_on(crate::client::fetch_feed())
            .expect("fetch failed");
        let mut count = 0;
        'outer: for entity in &feed.entity {
            let Some(tu) = entity.trip_update.as_ref() else {
                continue;
            };
            for stu in &tu.stop_time_update {
                let sid = stu.stop_id.as_deref().unwrap_or("");
                if sid == "1" || sid.to_uppercase().contains('T') {
                    let assigned = stu
                        .stop_time_properties
                        .as_ref()
                        .and_then(|p| p.assigned_stop_id.as_deref())
                        .unwrap_or("(none)");
                    println!(
                        "trip={} stop_id={sid:?} assigned={assigned:?} extracted={:?}",
                        tu.trip.trip_id.as_deref().unwrap_or("?"),
                        extract_track_from_stop_id(Some(if assigned != "(none)" {
                            assigned
                        } else {
                            sid
                        }))
                    );
                    count += 1;
                    if count >= 20 {
                        break 'outer;
                    }
                }
            }
        }
        println!("Total GCT/T-suffix stops sampled: {count}");
    }
}
