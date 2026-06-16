//! Simple structured logging helpers.
//!
//! Produces `[TIMESTAMP] [KNOT]` prefixed lines to stderr.
//! Volume is low (a few hundred events/day) so every event is logged.

/// Generate an ISO 8601 UTC timestamp string.
///
/// Converts Unix epoch seconds to Gregorian calendar date.
/// Uses a well-tested days-since-epoch to date conversion.
pub fn format_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hh, mm, ss
    )
}

/// Convert days since 1970-01-01 to (year, month, day).
///
/// Algorithm from "Calendrical Calculations" by Dershowitz & Reingold.
/// 1970-01-01 corresponds to civil day 719468.
fn days_to_ymd(days: u64) -> (i32, i32, i32) {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };

    (y as i32, m as i32, d as i32)
}

/// Log a notify event (raw file system event mapped to domain type).
pub fn log_notify_event(kind: &str, path: &std::path::Path, mapped: &str) {
    eprintln!(
        "[{}] [KNOT][NOTIFY] {} {} → {}",
        format_timestamp(),
        kind,
        path.display(),
        mapped,
    );
}

/// Log a config event being processed.
pub fn log_config_event(event: &str, detail: &str) {
    eprintln!("[{}] [KNOT][CONFIG] {event} — {detail}", format_timestamp());
}

/// Log a strand event being processed.
pub fn log_strand_event(event: &str, strand_path: &std::path::Path) {
    eprintln!(
        "[{}] [KNOT][STRAND] {event} — {}",
        format_timestamp(),
        strand_path.display(),
    );
}

/// Log a loom lifecycle event (register/unregister/discover).
pub fn log_loom_event(event: &str, loom_id: &str, detail: &str) {
    eprintln!(
        "[{}] [KNOT][LOOM] {event} loom={loom_id} — {detail}",
        format_timestamp()
    );
}

/// Log a knot lifecycle event (register/unregister/modify).
pub fn log_knot_event(event: &str, loom_id: &str, knot_id: &str, detail: &str) {
    eprintln!(
        "[{}] [KNOT][KNOT] {event} loom={loom_id} knot={knot_id} — {detail}",
        format_timestamp()
    );
}

/// Log a watch/unwatch operation.
///
/// `extra` is an optional detail string appended inside the parens
/// (e.g. `knot=my-knot` for Strand watches).
pub fn log_watch_event(action: &str, path: &std::path::Path, watch_type: &str, extra: Option<&str>) {
    let detail = match extra {
        Some(e) => format!("type={watch_type} {e}"),
        None => format!("type={watch_type}"),
    };
    eprintln!(
        "[{}] [KNOT][WATCH] {action} {} ({detail})",
        format_timestamp(),
        path.display(),
    );
}
