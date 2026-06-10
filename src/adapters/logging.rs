//! Simple structured logging helpers.
//!
//! Produces `[TIMESTAMP] [KNOT]` prefixed lines to stderr.
//! Volume is low (a few hundred events/day) so every event is logged.

/// Generate an ISO 8601 UTC timestamp string.
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
    let z = days_since_epoch as i64 + 719468;
    let a = z + 305;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hh, mm, ss
    )
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
pub fn log_watch_event(action: &str, path: &std::path::Path, watch_type: &str) {
    eprintln!(
        "[{}] [KNOT][WATCH] {action} {} (type={})",
        format_timestamp(),
        path.display(),
        watch_type,
    );
}
