# Plan: Spurious Delete Suppression — Burst Event Deduplication

## Related PRD

This plan contributes to [Spurious Delete Suppression — Burst Event Deduplication](../../prds/prd-spurious-delete-suppression.md).

This plan implements the 5-second DELETE suppression window in the debounce engine. When a DELETE event arrives, it is held in a pending suppression map for 5 seconds. If a CREATE or MODIFY for the same file arrives within that window, the DELETE is dropped (logged to rig-log) and only the CREATE/MODIFY is processed. After 5 seconds with no replacement, the DELETE is emitted as legitimate.

## Problem

When external tooling rewrites files using truncate+write (atomic save), inotify fires DELETE followed by CREATE or MODIFY ~1 second apart. The existing 100 ms same-kind debounce window is too narrow — the DELETE escapes the window, is emitted to the queue, and triggers a full agent invocation for a file that was actually just rewritten. A burst of 13 such rewrites produced 13 spurious agent invocations.

## Target

- `DebounceEngine` holds DELETE events in a pending suppression map (5-second window, configurable) before emitting them
- If a non-DELETE event arrives for the same `(path, loom, knot)` while a DELETE is pending, the DELETE is dropped and a `RigLogEvent::DeleteSuppressed` is logged
- After 5 seconds with no replacement, the DELETE is emitted as legitimate
- The existing 100 ms same-kind debounce window is unchanged
- Suppression window is configurable via `KNOT_DELETE_SUPPRESSION_S` env var (default 5 seconds, mirroring the test-debounce pattern)
- The debounce engine receives an `Arc<dyn RigLogPort>` for suppression logging

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `debounce.rs` module tests | Single event, rapid events, different files, delete_after_modify, same_file_different_knots, queue dedup, shutdown sentinel | ✅ 14 tests — green |
| `event_source.rs` module tests | Create/Modify/Delete mapping, directory filtering, config events | ✅ 10+ tests — green |
| `process_strand.rs` execution tests | Full pipeline for Created/Modified/Deleted events | ✅ 10+ tests — green |

## Test Gaps

- No test for DELETE followed by CREATE/MODIFY with gap > debounce window — this is the exact gap we need to fill
- No test for suppression logging to rig-log
- No integration test for burst event pattern against real inotify

## Phases

### Phase 0: `RigLogEvent::DeleteSuppressed` Domain Variant

Add a new `RigLogEvent::DeleteSuppressed` variant to the domain with fields: `strand_path`, `loom_id`, `knot_id`, `replacing_event` (the event kind that caused suppression: "Created" or "Modified"), and `timestamp`.

**Hexagonal layers:**
- **Domain** — `RigLogEvent::DeleteSuppressed` variant in `src/domain/events.rs` (derive `Serialize`, `Deserialize`, `Debug`, `Clone`, `PartialEq`, `Eq`)
- **Outbound adapter** — `FileSystemRigLog` handles the new variant automatically via serde (no code change needed)
- **Test** — Unit test verifying the variant serialises and deserialises correctly

### Phase 1: DELETE Suppression in DebounceEngine

Add the suppression window to `DebounceEngine::run()`.

**Architecture:**
- The debounce engine currently uses a single `pending: HashMap<(StrandPath, LoomId, KnotId), (StrandEvent, Instant)>` with a 100 ms deadline for all events
- Split this into two maps: `pending_normal` (100 ms, same behaviour as today) and `pending_delete` (suppression window, default 5 seconds)
- When a DELETE arrives → insert into `pending_delete` with suppression deadline. If already present, extend the deadline
- When a non-DELETE (CREATE/MODIFY) arrives → check `pending_delete` for the same key. If a DELETE is pending, log `RigLogEvent::DeleteSuppressed` to rig-log and remove from `pending_delete`. Then insert the non-DELETE into `pending_normal` as before
- Periodic check (5 ms tick) → emit expired entries from both maps independently

**Hexagonal layers:**
- **Port** — `RigLogPort` trait (already exists, no change needed — we use the existing `append` method)
- **Application** — `DebounceEngine` gains an `Option<Arc<dyn RigLogPort>>` parameter on `start()`, `start_with_window()`, `spawn_with_receiver()`, and `spawn_with_receiver_with_window()`. The `run()` loop implements the two-map suppression logic. Suppression window configurable via `KNOT_DELETE_SUPPRESSION_S` env var (default 5s, mirroring `KNOT_TEST_DEBOUNCE_MS` pattern)
- **Inbound** — `server.rs` passes `Arc::clone(&ctx.rig_log_port)` to the debounce engine spawn call
- **Test** — Unit tests in `debounce.rs`:
  - DELETE held in suppression window, emitted after expiry
  - DELETE suppressed when CREATE arrives within window (rig-log entry logged)
  - DELETE suppressed when MODIFY arrives within window (rig-log entry logged)
  - Multiple DELETEs for same file: suppression window extended on each new DELETE
  - Non-DELETE events unaffected (still use 100 ms same-kind debounce)
  - Different files suppressed independently

### Phase 2: Integration Test — Burst Event Dedup

End-to-end test using real inotify + filesystem to verify the burst suppression pattern.

**Hexagonal layers:**
- **Integration test** — `tests/debounce_burst.rs` (new file): creates temp directory with Notify watcher, writes 5 files using truncate+write pattern (atomic save simulation: `fs::write` which truncates), verifies that no DELETE events reach the queue and only CREATE/MODIFY events do, and checks that rig-log contains suppression entries. Uses `NotifyEventSource` + `DebounceEngine` wired together with a `MockRigLogPort` (in-memory) to capture suppression log entries.

## Notes

- The suppression window default of 5 seconds covers the observed ~1 second gap with 5× headroom. Configurable via env var for testing and edge cases.
- The approach does NOT use git diff to verify file state — this adds subprocess overhead and git may not be available. The suppression window alone is sufficient: if a file was truly deleted, no CREATE/MODIFY will arrive within 5 seconds.
- The existing InspectQueue dedup (same kind = replace, different kind = both) remains unchanged — suppression happens before the queue, so the queue never sees the spurious DELETEs at all.
- The `delete_after_modify` existing test (`debounce.rs`) sends Modify then Delete within 10 ms. Under the new logic, the Modify goes to `pending_normal` (100 ms window), the Delete goes to `pending_delete` (5s window). The Modify emits at 100 ms, the Delete emits at 5 seconds. This changes the test expectation — the test will need updating to account for the suppression window, or we accept that the existing test now exercises a different behaviour (Modify emitted quickly, Delete held then emitted after suppression window).
