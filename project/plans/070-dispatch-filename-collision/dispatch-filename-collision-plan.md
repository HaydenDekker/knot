# Plan: Unique Event Dispatch Filenames — Per-Batch Sequence Suffix

## Problem

When Knot parses a producer's tie-off and dispatches events, each event file is
named from the dispatch clock at **second precision**:

```
tie-offs/<rig>/<consumer-loom>/<EventId>/event-<YYYY-MM-DDTHH-MM-SS+ZZ>.md
```

(`FileSystemEventDispatcher::dispatch`, `src/adapters/outbound/event_dispatcher.rs:46` —
`format_timestamp()` → `timestamp.replace([':', ' '], "-")`.)

A collision needs all three conditions in one wall-clock second:

1. same event ID (same `{EventId}/` subdirectory),
2. same consumer loom (same dispatch directory),
3. same second (same filename).

All four writes then target one path, sequentially — **last writer wins**.

**Incident (2026-08-22 21:54:49):** `retest-validator`'s tie-off contained four
`ValidationFail` blocks (frontend, tauri-commands, tauri-desktop, tauri-android).
`dispatch_events_to_consumers` (`src/application/usecases/process_strand.rs:438`)
dispatched all four to the single subscriber `uat-gap-assessment` in one pass.
Four sequential writes hit `.../uat-gap-assessment-loom/ValidationFail/event-2026-08-22T21-54-49+01.md`;
only the fourth survived. The consumer's watcher saw one path and processed
exactly one strand. Three consumer-facing events were lost (analysis itself was
preserved in the producer's append-only tie-off).

The hazard is general: any knot that fans out the same event type to the same
consumer in one run (gap sweeps, per-item reports) will hit it. A secondary,
rarer path also collides: **two different consumer knots in the same loom**
subscribed to the same event from the same producer both dispatch into the same
`{loom}/{EventId}/` directory — even a single event then produces two writes to
one path.

A full scan of loom-log history found this to be the first occurrence, but the
precondition (N-way fan-out of one event type to one consumer) is a normal shape
for gap-sweep knots.

## Target

When this plan is complete:

1. **Batch sequencing.** One dispatch batch (one `dispatch_agent_events` call —
   i.e. one parsed tie-off, including event-enforcement follow-ups) assigns each
   dispatch a sequence position *within its target directory group*
   (group key = `consumer-loom-id` + `event-id`, which is exactly the
   `{loom}/{EventId}/` directory):

   | Group size | Filenames |
   |---|---|
   | 1 | `event-{ts}.md` (unchanged — no suffix) |
   | N > 1 | `event-{ts}-001.md`, `event-{ts}-002.md`, … `event-{ts}-NNN.md` |

   The suffix is a **3-digit zero-padded number** (1–999), so up to 999
   simultaneous dispatches to one directory are supported. Suffixes start at
   `001` (not `000`) so the plain name stays reserved for single-dispatch
   batches.

2. **Atomic creation (safety net).** Event files are created with
   `OpenOptions::create_new(true)` instead of `std::fs::write` (which
   unconditionally overwrites). If the computed name is already taken —
   a leftover from an earlier run in the same second, or a future concurrent
   dispatch — the adapter increments the suffix until it finds a free name
   (bounded retry; a bounded error rather than silent overwrite if exhausted).
   This makes "two writes, one path" impossible, not just unlikely.

3. **Delivery traceability.** The `EventsDispatched` loom-log entry records the
   created file path per dispatch — its `dispatches` array extends from
   `(event-id, consumer-knot-id, consumer-loom-id)` to
   `(event-id, consumer-knot-id, consumer-loom-id, file-path)`. After a fan-out,
   the loom-log shows exactly which file each event produced.

4. **Consumers unchanged.** Event-file detection is prefix-based
   (`filename.starts_with("event-")` in `strand_event_metadata.rs` and
   `context_providers.rs`) plus frontmatter — the suffix is transparent. The
   notify watcher reports per path, so N distinct files in the same second
   produce N distinct strands, each processed independently.

Naming example (the incident, replayed):

```
tie-offs/<rig>/uat-gap-assessment-loom/ValidationFail/
├── event-2026-08-22T21-54-49+01-001.md   ← frontend
├── event-2026-08-22T21-54-49+01-002.md   ← tauri-commands
├── event-2026-08-22T21-54-49+01-003.md   ← tauri-desktop
└── event-2026-08-22T21-54-49+01-004.md   ← tauri-android
```

## Design Notes (Hexagonal)

- **Domain** — `LoomEvent::EventsDispatched` (`src/domain/events.rs:443`):
  `dispatches: Vec<(String, String, String)>` → `Vec<(String, String, String,
  String)>` (4th element = created file path, absolute or rig-relative — decide
  in phase, prefer absolute to match other path-carrying events). Precedent:
  the 2→3 tuple expansion shipped in binary 0.30.1 with a `knot-update`
  changelog entry; the loom-log reader already skips unparseable lines with a
  warning, so old 3-tuple entries degrade gracefully (warning noise only, no
  data loss).
- **Application — port** — `EventDispatcherPort::dispatch`
  (`src/application/ports.rs:648`) gains a `seq: u32` parameter:
  `0` = plain name (single-dispatch group), `i ≥ 1` = `-{i:03}` suffix. The use
  case owns the batch-sequencing policy; the adapter owns name materialisation.
- **Application — use case** — `ProcessStrand::dispatch_events_to_consumers`
  (`src/application/usecases/process_strand.rs:438`) becomes two-pass:
  1. collect all `(event, loom, consumer_knot)` matches (existing scan),
  2. group by `(consumer_loom_id, event.event_id)`, dispatch each group with
     `seq = 0` if the group has one member, else `seq = 1..N`.
  Dispatch order within a group follows tie-off block order (insertion order),
  so suffix order matches the producer's emission order.
- **Adapter** — `FileSystemEventDispatcher` (`src/adapters/outbound/event_dispatcher.rs`):
  pure helper `event_file_name(timestamp: &str, seq: u32) -> String`
  (unit-testable without a clock) + a resolve loop: compute candidate name from
  `seq`, `create_new` it, on `AlreadyExists` bump the suffix (from `seq + 1`, or
  from 1 when `seq == 0`) and retry up to a cap (1000) → `PortError` if
  exhausted. Content is written to the opened handle and flushed.
- **Inbound adapters** — no changes (HTTP/state surface unchanged; the
  `EventsDispatched` JSON shape change is internal to the loom-log artifact).

## Existing Tests

| Test | What it covers | Status |
|---|---|---|
| `event_dispatcher.rs` — `dispatch_creates_event_file_with_correct_path` | Path prefix, `event-` filename prefix, `.md` extension | ✅ Green — pins current naming (no suffix) |
| `event_dispatcher.rs` — `dispatch_timestamp_in_filename_is_sane` | No colons in filename | ✅ Green |
| `event_dispatcher.rs` — `dispatch_fan_out_two_consumers_same_event` | Same event → two consumer *looms* | ✅ Green — different dirs, no collision exercised |
| `event_dispatcher.rs` — `dispatch_fan_out_same_loom_different_event_ids` | Same loom, two event *ids* | ✅ Green — different dirs, no collision exercised |
| `process_strand.rs` — `event_dispatch_full_flow`, `event_dispatch_fan_out_two_looms`, `no_events_no_dispatch`, `event_occurred_false_produces_no_dispatch`, `event_id_mismatch_no_dispatch` | Parse → match → dispatch via `MockEventDispatcher` + `EventsDispatched` log entry | ✅ Green — mock records no sequence |
| `tests/pipeline.rs` | Full `ProcessStrand` flow (single-event cases) | ✅ Green — always wires `MockEventDispatcher` |
| `tests/event_enforcement.rs` | Event enforcement / follow-up dispatch | ✅ Green |

## Test Gaps

- No test dispatches **multiple events to the same consumer directory in the
  same second** — the exact incident scenario is untested.
- No test asserts that **all N event payloads are delivered** (the
  last-writer-wins regression: today only the last file's content is verifiable).
- No test covers **two consumer knots in one loom** subscribed to the same
  event (secondary collision path).
- No test exercises the **name-taken fallback** (a file with the computed name
  already existing).
- `MockEventDispatcher` records no sequence — the use case's seq assignment is
  unobservable to tests today.
- No acceptance test drives the **consumer side** of a multi-event fan-out
  (watcher → N strands → N processings).

## Phases

### Phase 0: Filename derivation helper (TDD)

Extract the filename computation into a pure, clock-free helper
`event_file_name(timestamp: &str, seq: u32) -> String` in
`src/adapters/outbound/event_dispatcher.rs`: `seq == 0` → `event-{ts}.md`
(current behaviour, `:`/space replaced by `-`); `seq ≥ 1` →
`event-{ts}-{seq:03}.md`. Failing tests first (seq 0/1/2/999, colon
substitution preserved), then implement; refactor `dispatch()` to call the
helper with `seq = 0` so on-disk names are byte-identical to today's.

### Phase 1: Port + use-case sequencing

Change `EventDispatcherPort::dispatch` to accept `seq: u32` (docs: 0 = plain
name, `i ≥ 1` = `-{i:03}` suffix); update `MockEventDispatcher` to record it
(extend its recorded tuple) and fix existing destructuring in
`process_strand.rs` tests. Rewrite `dispatch_events_to_consumers` as
collect-then-group-then-dispatch, assigning per-group sequences in tie-off
block order. Tests:

- 4 same-id events → 1 consumer: mock records `seq` 1,2,3,4; with the real
  dispatcher, 4 distinct files exist **and each file contains its own
  `description`/payload** (the incident regression — all four survive).
- 1 event → 2 consumer knots in the same loom: `seq` 1,2, distinct files.
- Existing fan-out tests keep passing with `seq = 0` (plain names, no suffix).
- Group size 1 across different dirs (different event id / different loom)
  stays `seq = 0`.

### Phase 2: Atomic creation + taken-name fallback

Replace `std::fs::write` with a `create_new` resolve loop in
`FileSystemEventDispatcher` (bump suffix on `AlreadyExists`, bounded retry,
`PortError` on exhaustion; write + flush the opened handle). Deterministic unit
tests using an injected timestamp string (no real clock): pre-create
`event-{ts}.md` → `seq = 0` lands on `event-{ts}-001.md`; pre-create `-001`
→ lands on `-002`; exhaustion cap returns a clear error. Verify no test
depends on overwrite semantics.

### Phase 3: Acceptance test — producer fan-out to consumer processing

Extend `tests/helpers.rs` `ProcessStrandBuilder` with an option to wire the
**real** `FileSystemEventDispatcher` (currently always mocked). Add an
acceptance test (in `tests/pipeline.rs` or a new `tests/event_fanout.rs`) that
replays the incident end-to-end:

1. Producer loom + knot; mock agent returns a tie-off with **four
   `ValidationFail` blocks** (distinct CI payloads, one shared event id).
2. Consumer loom + one knot subscribed to `ValidationFail`.
3. Execute the producer strand → assert **4 event files** in
   `tie-offs/<rig>/<consumer-loom>/ValidationFail/` with 4 distinct names and
   4 distinct payloads; assert the `EventsDispatched` loom-log entry lists 4
   dispatches.
4. Drive the consumer side: for each created event file, execute a
   `StrandEvent::Created` against the consumer knot → assert the consumer
   processed **4 strands** (4 `KnotCompleted` entries; event metadata
   traceable via `extract_event_metadata` on each event file).

### Phase 4: Observability + documentation

Extend `LoomEvent::EventsDispatched.dispatches` to the 4-tuple (add created
file path), populated from the path returned by `dispatch()`; update
`dispatch_agent_events`/`dispatch_events_to_consumers` to carry paths through.
Add a loom-log test that a legacy 3-tuple line is skipped with a warning while
subsequent lines read fine (pins the 0.30.1-style graceful degradation). Update
doc comments for the new filename contract (`ports.rs` port docs,
`event_dispatcher.rs` module docs, `events.rs` event docs). Add a `knot-update`
changelog entry for the `EventsDispatched` tuple expansion (internal runtime
artifact; migration = none required, warnings self-settle) and note the new
`event-{ts}-NNN.md` filename shape for fan-out batches.

## Notes

- **Separator choice.** The suffix uses a hyphen (`event-{ts}-001.md`),
  consistent with the existing hyphen-heavy filename and keeping one parseable
  shape: `event-<anything>[-NNN].md`. (Underscore was suggested; either works —
  hyphen preferred for consistency. Decide and record in the phase doc.)
- **Sorting.** In a mixed second, the plain name sorts before `-001…` (it is a
  prefix of them), so single events precede batch siblings alphabetically —
  harmless, and batch siblings sort in emission order.
- **Why not sub-second precision in the timestamp.** It would fix same-second
  collisions but not same-*millisecond* ones, keeps the name fragile to clock
  behaviour, and makes the name depend on an implementation detail of *when the
  write happened* rather than the deterministic batch position. Sequencing is
  explicit; atomicity closes the remaining race.
- **999 cap.** A group larger than 999 dispatches fails with a clear
  `PortError` (dispatch is best-effort; the producer tie-off retains all
  events). Not realistically reachable.
- **Out of scope.** The create→write→flush gap (a watcher could observe an
  empty file mid-write) pre-exists this plan and is mitigated by the debounce
  window; rename-based atomic publish would be a separate change. Consumer
  code, git commit subjects (cosmetic strand-name usage), and the HTTP surface
  are unchanged.
- **Binary version.** The `EventsDispatched` shape change ships with the next
  version bump via plan completion; `knot-update` changelog is the record.
