# Phase 2: Atomic creation + taken-name fallback

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [x] Replace `std::fs::write` in `FileSystemEventDispatcher::dispatch` with `OpenOptions::new().write(true).create_new(true)` resolve loop — extracted as `pub(crate) fn create_event_file(event_dir, timestamp, seq, content)` on `FileSystemEventDispatcher`; on `AlreadyExists` the suffix bumps by one (seq 0 → plain, -001, -002 …; seq N → -NNN, -NNN+1 …); non-collision I/O errors return immediately (not retried)
- [x] Bounded retry: `const MAX_NAME_RETRIES: u32 = 1000` (1001 total attempts); `PortError::EventDispatchFailed("could not allocate a unique event filename in '<dir>' after 1000 retries")` on exhaustion; `checked_add` guards u32 overflow
- [x] Content is written to the opened handle and flushed before the path is returned
- [x] Deterministic test (injected timestamp, no real clock): pre-create `event-{ts}.md` → `seq = 0` lands on `event-{ts}-001.md` — `create_event_file_taken_plain_name_falls_back_to_001` (also asserts the pre-existing file is untouched)
- [x] Deterministic test: pre-create `event-{ts}-001.md` → `seq = 1` lands on `event-{ts}-002.md` — `create_event_file_taken_seq_name_falls_back_to_next`
- [x] Deterministic test: exhaustion cap returns a clear `PortError` — `create_event_file_exhausts_retries_and_errors` (pre-creates all 1001 candidates; asserts error message and that no extra file appeared)
- [x] Bonus tests: fresh dir `seq = 0` uses plain name + content round-trips; non-collision I/O error (file where a directory should be) fails immediately — `create_event_file_fresh_dir_uses_plain_name`, `create_event_file_io_error_is_not_retried`
- [x] Verify no existing test depends on overwrite semantics — audited all `FileSystemEventDispatcher` users: only `server.rs` (production wiring) and the adapter's own tests; no test dispatches twice to the same `{loom}/{EventId}/` directory (all use distinct event ids or looms), so `create_new` changes nothing for them
- [x] Compile and verify no errors
- [x] Run full test suite — lib 810 passed (1 pre-existing failure), all integration targets pass

## Deviations
- The resolve loop lives in a `create_event_file()` helper rather than inline in `dispatch()` — needed so the deterministic tests can inject a timestamp string (`dispatch()` itself still calls the real clock; the helper is the test seam). Matches the plan's "deterministic unit tests using an injected timestamp string" requirement.

## Discoveries
- Bump semantics unify cleanly: `candidate = seq`, then `candidate += 1` on each collision. For `seq = 0` that walks plain → -001 → -002 …; for `seq = N` it continues N → N+1 → … — exactly the plan's "from `seq + 1`, or from 1 when `seq == 0`".
- A suffix can exceed 999 during fallback (e.g. `-1000`) if a pathological directory has taken the first 1000 names — still unique and parseable (`event-<anything>[-NNN].md`); the 999 cap in the plan governs the *use-case-assigned* batch position, not the fallback walk.

## Notes
- `create_new` makes "two writes, one path" impossible: the second writer gets `AlreadyExists` and moves on. The create→write→flush window (watcher could see an empty file mid-write) pre-exists and is out of scope per the plan (debounce window mitigates; rename-based publish would be a separate change).
