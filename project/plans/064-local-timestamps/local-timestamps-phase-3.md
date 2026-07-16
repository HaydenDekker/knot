# Phase 2: Update domain doc comments and prompt template

## Tasks

- [x] Update `src/domain/events.rs`: all `/// ISO 8601 UTC timestamp.` → `/// ISO 8601 timestamp (local time).`
- [x] Update `src/domain/entities.rs`: UTC references → local time
- [x] Update `src/application/debounce.rs`: UTC reference → local time
- [x] Update `src/application/usecases/types.rs`: `format_timestamp()` doc comment
- [x] Update `src/application/usecases/context_providers.rs`: `PendingEvent.timestamp` doc comment
- [x] Update prompt template in `build_listener_context()` — format example now says `<ISO 8601 timestamp (local time)>`

## Result

All documentation and prompt text now reference "local time" consistently. No behavioural change — purely documentation alignment with the new timestamp format.

## Verification

- `cargo test --lib events` — 105 tests pass
- No tests asserted on "UTC" text (no test changes needed)
