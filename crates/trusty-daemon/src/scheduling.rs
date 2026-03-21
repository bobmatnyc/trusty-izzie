//! Scheduling helpers for computing future Unix timestamps.

/// Returns Unix timestamp for the next occurrence of HH:MM local time.
pub fn next_time_of_day_ts(hour: u32, minute: u32) -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    let today_target = now.date_naive().and_hms_opt(hour, minute, 0).unwrap();
    let today_local = Local.from_local_datetime(&today_target).earliest().unwrap();
    if today_local > now {
        today_local.timestamp()
    } else {
        (today_local + chrono::Duration::days(1)).timestamp()
    }
}

/// Returns Unix timestamp `interval_minutes` from now.
pub fn next_interval_ts(interval_minutes: u32) -> i64 {
    let now = chrono::Local::now();
    (now + chrono::Duration::minutes(interval_minutes as i64)).timestamp()
}

/// Returns Unix timestamp for the next occurrence of `weekday` at HH:MM local time.
pub fn next_weekly_ts(weekday: chrono::Weekday, hour: u32, minute: u32) -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    let now = Local::now();
    let today_weekday = now.weekday();
    let days_until = (weekday.num_days_from_monday() as i64
        - today_weekday.num_days_from_monday() as i64)
        .rem_euclid(7);
    let target_date = now.date_naive() + chrono::Duration::days(days_until);
    let target_naive = target_date.and_hms_opt(hour, minute, 0).unwrap();
    let target_local = Local.from_local_datetime(&target_naive).earliest().unwrap();
    if target_local > now {
        target_local.timestamp()
    } else {
        (target_local + chrono::Duration::weeks(1)).timestamp()
    }
}
