# Phase 0: Add `chrono` dependency and replace `logging.rs` timestamp

## Tasks

- [x] Add `chrono = "0.4"` to `Cargo.toml` dependencies
- [x] Replace `format_timestamp()` in `src/adapters/logging.rs` with `chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()`
- [x] Remove `days_to_ymd()` from `logging.rs`
- [x] Add unit test asserting output contains timezone offset (not `Z`)
- [x] Add unit test asserting ISO 8601 shape (25 chars, correct separators)

## Result

`format_timestamp()` now produces local-time timestamps like `2026-07-17T14:30:00+01:00`. Two new unit tests pass. Function signature preserved — no callers need updating.

## Verification

- `cargo test --lib logging` — 2 tests pass
- `cargo build` — clean
