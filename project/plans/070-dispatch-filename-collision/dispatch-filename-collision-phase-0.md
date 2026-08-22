# Phase 0: Filename derivation helper (TDD)

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [x] Write failing tests for `event_file_name(timestamp, seq)` in `src/adapters/outbound/event_dispatcher.rs`: `seq = 0` → `event-{ts}.md`, `seq = 1`/`2`/`999` → `event-{ts}-{seq:03}.md`, colon/space substitution preserved — 6 tests, confirmed red (E0425) before implementation
- [x] Implement pure helper `pub(crate) fn event_file_name(timestamp: &str, seq: u32) -> String` — free function at module top level, takes the raw timestamp string and owns the `:`/space → `-` replacement
- [x] Refactor `dispatch()` to call the helper with `seq = 0` — on-disk names byte-identical to today (existing path/filename tests all green)
- [x] Compile and verify no errors
- [x] Run full lib test suite — 802 passed, 1 failed (pre-existing `build_listener_context_prompt_includes_do_not_edit_guidance`, fails on clean main — unrelated prompt-wording assertion)

## Deviations
- None.

## Discoveries
- The raw timestamp shape is `%Y-%m-%dT%H:%M:%S%:z` (e.g. `2026-08-22T21:54:49+01:00`), so a local-timezone filename actually ends `+01-00` — the plan's `+ZZ` example is a simplification. The helper applies to whatever string `format_timestamp()` returns, so this needs no special handling.

## Notes
- Helper is `pub(crate)` (not `pub`) — only the adapter and its tests use it; the port contract is what the application layer sees.
- Keeping the replacement inside the helper means `dispatch()` no longer touches `str::replace` — one owner for the filesystem-safety rule.
