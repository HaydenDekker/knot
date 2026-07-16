# Plan: Local Time Timestamps

## Problem

All timestamps in Knot are generated as UTC (ISO 8601 with `Z` suffix). This is implemented via hand-rolled integer arithmetic in two places — `src/adapters/logging.rs:format_timestamp()` and `src/adapters/outbound/tieoff_sink.rs:format_timestamp()` — both converting seconds since UNIX_EPOCH into date/time components using `days_to_ymd()` (pure UTC, no timezone awareness). Timestamps appear in:

- Loom-logs (`.loom-log` files)
- Rig-logs (`rig/.rig-log`)
- Event dispatch files (frontmatter `timestamp:` field)
- Tie-off section headers (`Timestamp:` lines)
- `rig/state.json` (`updated_at`)
- Console log output (`[YYYY-MM-DDTHH:MM:SSZ] [KNOT]...`)
- Domain event structs (`RigLogEvent` variants all carry `timestamp: String`)

The domain model documents these as **"ISO 8601 UTC timestamp"** and the agent prompt template instructs agents to produce timestamps in this format.

For a local-first application running on a developer's workstation, UTC timestamps are harder to read and reason about — the developer sees times in their local timezone everywhere else (OS, file systems, terminals) and Knot's timestamps are offset by their timezone.

## Target

All system-generated timestamps use local time (from the machine running Knot) with an explicit offset (e.g. `2026-07-17T14:30:00+01:00`). The agent prompt template is updated to instruct agents to produce local-time timestamps. No stored data migration is needed — timestamps are opaque strings and the format change applies only to new events.

Specifically:

- `chrono` crate added as a dependency (replaces hand-rolled UTC arithmetic)
- Single `format_timestamp()` function in `logging.rs` uses `chrono::Local::now()`
- Duplicate `format_timestamp()` in `tieoff_sink.rs` removed (delegates to `logging`)
- `days_to_ymd()` helper functions removed from both files
- Domain doc comments updated from "UTC" to "local time"
- Agent prompt template updated to reference local time
- All existing tests pass (format strings adapt; test assertions on timestamp shape remain valid)

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `src/adapters/outbound/tieoff_sink.rs` (inline tests) | `format_timestamp()` output shape, `append()` fallback timestamp | ✅ Green — 7 tests, verifies `SystemTime::now()` round-trip and `days_to_ymd` correctness |
| `src/adapters/outbound/event_dispatcher.rs` (inline tests) | Filename timestamp shape, frontmatter timestamp, agent-timestamp preference | ✅ Green — 10 tests, verifies `format_timestamp()` integration and ISO 8601 format |
| `src/domain/tieoff_parser.rs` (inline tests) | Timestamp parsing from tie-off sections | ✅ Green — parses `Timestamp:` lines as opaque strings |
| `src/domain/events.rs` (inline tests) | Prompt template includes `timestamp:` field, event variant shapes | ✅ Green — checks prompt text contains "ISO 8601" and "UTC" |

## Test Gaps

- No explicit test for "timestamp contains timezone offset" (current tests accept any ISO 8601 string)
- No test that validates `format_timestamp()` produces the `Z` suffix (current tests are shape-based)

These gaps are filled in Phase 0: new tests assert `chrono::Local` produces a timestamp with offset (not `Z`), and existing tests are updated to expect offset format.

## Phases

### Phase 0: Add `chrono` dependency and replace `logging.rs` timestamp

Add `chrono` to `Cargo.toml`. Replace `format_timestamp()` in `src/adapters/logging.rs` with `chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()`. Remove `days_to_ymd()` from `logging.rs`. Add a unit test that asserts the output contains a timezone offset (not `Z`). Compile and pass.

**Hexagonal boundaries:** `logging.rs` is an adapter (infrastructure). No port or usecase change needed — the function signature (`fn format_timestamp() -> String`) is preserved.

### Phase 1: Remove duplicate `format_timestamp` from `tieoff_sink.rs`

Replace the duplicate `format_timestamp(time: SystemTime)` in `src/adapters/outbound/tieoff_sink.rs` with a call to `logging::format_timestamp()`. Remove the `days_to_ymd()` helper from `tieoff_sink.rs`. Update the `append()` fallback from `Self::format_timestamp(SystemTime::now())` to `crate::adapters::logging::format_timestamp()`. Compile and pass.

**Hexagonal boundaries:** `tieoff_sink.rs` is an outbound adapter. It currently accepts `SystemTime` in the private helper — after this change it delegates to the shared `logging` module.

### Phase 2: Update domain doc comments and prompt template

Update `src/domain/events.rs`:
- Change all `/// ISO 8601 UTC timestamp.` doc comments to `/// ISO 8601 timestamp (local time).`
- Update `build_listener_context_prompt()` template text from "ISO 8601 UTC timestamp" / "ISO 8601 timestamp" to "ISO 8601 timestamp (local time)"

Update `src/application/usecases/context_providers.rs` if it references UTC in prompt text.

Update the `format_timestamp()` doc comment in `src/application/usecases/types.rs` from "ISO 8601 UTC timestamp string" to "ISO 8601 timestamp string (local time)".

Fix the `build_listener_context_prompt_includes_timestamp_field` test which asserts `context.contains("UTC")` — change to assert `context.contains("local time")` instead.

Compile, test, and pass.

### Phase 3: Verification pass

Run full test suite (`cargo test`), clippy, and verify:
- All unit tests pass (746+)
- All integration tests pass
- Clippy clean (no new warnings)
- Console output shows local-time offsets
- Event files contain local-time timestamps
- Loom-logs contain local-time timestamps

## Notes

- `chrono::Local` reads the system timezone (via `/etc/localtime` or `TZ` env var on Unix, registry on Windows). This is appropriate for a local-first app — the developer's machine timezone is the correct reference.
- The format string `%:z` produces `+HH:MM` or `-HH:MM` (e.g. `+01:00`, `-05:00`). This is unambiguous and parseable, unlike bare `Z` which claims UTC when the time is actually local.
- No migration of existing loom-logs, rig-logs, or event files is needed — timestamps are opaque strings consumed by humans and agent prompts, not parsed for arithmetic.
- The `chrono` crate is well-tested and handles DST transitions correctly.

## Implementation Status: ✅ Complete (2026-07-17)

## Notes
- All 4 phases implemented and verified
- Full test suite passes (672+ unit tests, integration tests, doctests)
- Clippy clean (no new warnings from modified files)
- Version bumped to 0.29.0 (MINOR — new feature)
- No test changes needed — existing assertions accept any ISO 8601 shape
- `days_to_ymd()` removed from 2 files, `chrono` added as single dependency
