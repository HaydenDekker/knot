# Phase 1: Remove duplicate `format_timestamp` from `tieoff_sink.rs`

## Tasks

- [x] Replace `Self::format_timestamp(SystemTime::now())` with `logging::format_timestamp()` in `append()` fallback
- [x] Remove `format_timestamp(time: SystemTime)` method from `FileSystemTieOffSink`
- [x] Remove `days_to_ymd()` helper from `FileSystemTieOffSink`
- [x] Remove `use std::time::SystemTime` import
- [x] Add `use crate::adapters::logging` import

## Result

Single source of truth for timestamp formatting in `logging::format_timestamp()`. Tie-off sink delegates to it. All 8 tieoff_sink tests pass.

## Verification

- `cargo test --lib tieoff_sink` — 8 tests pass
- `cargo build` — clean
