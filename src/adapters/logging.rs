//! Simple structured logging helpers.
//!
//! Produces `[KNOT]` prefixed lines to stderr.
//! Volume is low (a few hundred events/day) so every event is logged.

/// Log a notify event (raw file system event mapped to domain type).
pub fn log_notify_event(kind: &str, path: &std::path::Path, mapped: &str) {
    eprintln!(
        "[KNOT][NOTIFY] {} {} → {}",
        kind,
        path.display(),
        mapped,
    );
}

/// Log a config event being processed.
pub fn log_config_event(event: &str, detail: &str) {
    eprintln!("[KNOT][CONFIG] {event} — {detail}");
}

/// Log a strand event being processed.
pub fn log_strand_event(event: &str, strand_path: &std::path::Path) {
    eprintln!(
        "[KNOT][STRAND] {event} — {}",
        strand_path.display(),
    );
}

/// Log a loom lifecycle event (register/unregister/discover).
pub fn log_loom_event(event: &str, loom_id: &str, detail: &str) {
    eprintln!("[KNOT][LOOM] {event} loom={loom_id} — {detail}");
}

/// Log a knot lifecycle event (register/unregister/modify).
pub fn log_knot_event(event: &str, loom_id: &str, knot_id: &str, detail: &str) {
    eprintln!(
        "[KNOT][KNOT] {event} loom={loom_id} knot={knot_id} — {detail}",
    );
}

/// Log a watch/unwatch operation.
pub fn log_watch_event(action: &str, path: &std::path::Path, watch_type: &str) {
    eprintln!(
        "[KNOT][WATCH] {action} {} (type={})",
        path.display(),
        watch_type,
    );
}
