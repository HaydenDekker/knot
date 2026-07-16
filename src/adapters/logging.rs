//! Simple structured logging helpers.
//!
//! Produces `[TIMESTAMP] [KNOT]` prefixed lines to stderr.
//! Volume is low (a few hundred events/day) so every event is logged.

/// Generate an ISO 8601 timestamp string in local time.
///
/// Uses the system timezone (e.g. `/etc/localtime` on Unix, registry on
/// Windows). Output includes the timezone offset (e.g. `+01:00`, `-05:00`).
pub fn format_timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `format_timestamp()` produces ISO 8601 with a timezone offset
    /// (not the `Z` UTC suffix).
    #[test]
    fn format_timestamp_contains_timezone_offset() {
        let ts = format_timestamp();
        // Shape: YYYY-MM-DDTHH:MM:SS+HH:MM or YYYY-MM-DDTHH:MM:SS-HH:MM
        assert!(
            ts.contains('+') || ts.contains("-T"),
            "timestamp should contain a timezone offset (+ or - after T), got: {}",
            ts
        );
        // Must NOT end with 'Z' (that would be UTC)
        assert!(
            !ts.ends_with('Z'),
            "timestamp should NOT end with Z (UTC), got: {}",
            ts
        );
    }

    /// `format_timestamp()` output matches ISO 8601 date-time pattern.
    #[test]
    fn format_timestamp_matches_iso8601_shape() {
        let ts = format_timestamp();
        // Example: 2026-07-17T14:30:00+01:00
        assert!(
            ts.len() == 25,
            "timestamp should be 25 chars (YYYY-MM-DDTHH:MM:SS±HH:MM), got {} chars: {}",
            ts.len(),
            ts
        );
        // Date portion: YYYY-MM-DDT
        assert!(ts.chars().nth(4) == Some('-'));
        assert!(ts.chars().nth(7) == Some('-'));
        assert!(ts.chars().nth(10) == Some('T'));
        // Time portion: HH:MM:SS
        assert!(ts.chars().nth(13) == Some(':'));
        assert!(ts.chars().nth(16) == Some(':'));
    }
}
