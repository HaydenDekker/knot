# Design: Event Dispatch — Unique Filenames for Same-Second Fan-Out

**Type:** Implementation pattern
**Subsystem:** outbound event dispatch (`FileSystemEventDispatcher`)

## What It Is

When a producer knot's tie-off dispatches events to consumer knots, each
dispatch creates a file in the consumer's dispatch directory
(`tie-offs/<rig>/<consumer-loom>/<EventId>/`). Filenames are derived from
the dispatch clock at **second precision**, so any fan-out that targets
the *same directory* within the *same second* (multiple blocks of one
event id in one tie-off, or two consumer knots in one loom subscribing to
the same event) would compute the *same filename* — and with a plain
`fs::write`, the last writer silently won. The incident this pattern
fixes (2026-08-22): a four-way `ValidationFail` fan-out delivered one
file; three events were lost to the consumer (the producer's append-only
tie-off retained them).

The pattern makes unique names **deterministic** (per-batch sequence
suffix) and makes overwrites **impossible** (atomic creation with a
taken-name fallback) — the two mechanisms are complementary: sequencing
gives batch siblings stable, order-meaningful names; atomicity closes
the residual race between separate batches that land in the same second.

## The Pattern

### 1. Deterministic batch sequencing (naming policy)

A *dispatch batch* is one `dispatch_agent_events` call — one parsed
tie-off, including event-enforcement follow-ups. Within a batch, each
dispatch is grouped by its **target directory** (`consumer_loom_id` +
`event_id` — exactly the `{loom}/{EventId}/` directory) and assigned a
sequence position within its group:

| Group size | Filenames |
|---|---|
| 1 | `event-{ts}.md` (plain name — no suffix) |
| N > 1 | `event-{ts}-001.md`, `event-{ts}-002.md`, … `event-{ts}-NNN.md` |

- The suffix is a **3-digit zero-padded number starting at 1** (never 0),
  so the plain name stays reserved for single-dispatch batches.
- Suffix order follows the batch's match order — tie-off block order for
  the events, loom-store order for the consumers — so filenames mirror
  the producer's emission order.
- Cap: a group larger than 999 dispatches is not realistically
  reachable; the fallback loop (below) can still walk past 999
  (`-1000`, …) if a pathological directory has taken the first names —
  names stay unique and parseable (`event-<anything>[-NNN].md`).

The **use case owns the sequencing policy** (collect matches → group →
assign seq); the **adapter owns name materialisation** (seq → filename
string). The boundary is the port's `seq: u32` parameter: `0` = plain
name, `i ≥ 1` = `-{i:03}` suffix.

### 2. Atomic creation with taken-name fallback (uniqueness guarantee)

The adapter creates files with `OpenOptions::new().write(true)
.create_new(true)` — never an unconditional write. If the computed name
is already taken (leftover from an earlier run in the same second, or a
concurrent dispatch), the suffix is **bumped by one and retried** — up
to a bounded cap (1000 retries), then a clear `PortError`. The content
is written to the opened handle and flushed before the path is returned.

Result: "two writes, one path" is impossible, not just unlikely. The
fallback also means a *stale file* from a crashed same-second run can
never mask a new event.

### 3. Delivery traceability (observability)

The `EventsDispatched` loom-log entry records the created file path per
dispatch — its `dispatches` array is
`(event-id, consumer-knot-id, consumer-loom-id, created-file-path)`.
After a fan-out, the loom-log shows exactly which file each event
produced. The 4th element was added in Knot 0.33.0; the loom-log reader
skips legacy 3-tuple lines with a warning (0.30.1-style graceful
degradation — warning noise only, no data loss).

### 4. Consumers stay unchanged

Event-file detection is prefix-based (`filename.starts_with("event-")`)
plus frontmatter, so the suffix is transparent. The notify watcher
reports per path, so N distinct files in the same second produce N
distinct strands, each processed independently.

## Filename Contract

```
tie-offs/<rig>/<consumer-loom>/<EventId>/event-{ts}[-NNN].md
```

- `{ts}` — dispatch clock at second precision
  (`%Y-%m-%dT%H:%M:%S%:z`), `:` and space replaced with `-`.
- `[-NNN]` — 3-digit zero-padded batch sequence (1–999), absent when
  `seq = 0`.
- The separator is a **hyphen** (consistent with the existing
  hyphen-heavy filename; one parseable shape). In a mixed second the
  plain name sorts before its `-001…` batch siblings (it is a prefix of
  them) — harmless: single events precede batch siblings
  alphabetically, and batch siblings sort in emission order.

## Hexagonal Layering (Knot)

| Layer | Piece | Responsibility |
|---|---|---|
| Application — use case | `ProcessStrand::dispatch_events_to_consumers` | Collect `(event, loom, consumer_knot)` matches; group by target directory; assign per-group seq; carry created paths into the `EventsDispatched` entry |
| Application — port | `EventDispatcherPort::dispatch(…, seq: u32)` | Contract: `0` = plain name, `i ≥ 1` = `-{i:03}` suffix; adapter guarantees a unique file |
| Domain | `LoomEvent::EventsDispatched` | 4-tuple `dispatches` (path = absolute, as returned by the dispatcher) |
| Adapter | `FileSystemEventDispatcher` | `event_file_name(ts, seq)` (pure, clock-free); `create_event_file(dir, ts, seq, content)` (create_new resolve loop, `MAX_NAME_RETRIES = 1000`); content build |

`event_file_name` is a free `pub(crate)` function in the adapter — the
port contract (seq semantics) is what the application layer sees; the
string shape is an adapter detail.

## Why Not Sub-Second Timestamps

Adding milliseconds (or nanoseconds) to the timestamp would fix
same-*second* collisions but not same-*millisecond* ones, keeps the name
fragile to clock behaviour, and makes the name depend on an
implementation detail of *when the write happened* rather than the
deterministic batch position. Explicit sequencing + atomicity is the
robust combination.

## Gotchas

- **The create→write→flush window pre-exists this pattern.** A watcher
  could observe a newly created (but not yet fully written) file mid-
  write. The debounce window mitigates it; rename-based atomic publish
  would be a separate change.
- **Legacy loom-log lines are skipped, not migrated.** A 0.32.x
  `EventsDispatched` line (3-tuple) fails to deserialize into the
  4-tuple and is skipped with a warning on every read until the log is
  rotated/compacted — by design (same precedent as the 2→3 tuple
  expansion in 0.30.1).
- **The mock dispatcher returns a synthetic path**
  (`…/event-mock.md`) — tests asserting on the 4th tuple element must
  expect that shape, not a real timestamped name.
- **`seq = 0` is overloaded:** it means "singleton group, plain name"
  in the use case, and "try the plain name first, then fall back to
  `-001`, `-002`, …" in the adapter. The fallback start (`seq + 1`, or
  `1` when `seq == 0`) follows from a single rule: bump the candidate by
  one on each collision.

## Verification

| Test | Level | Pins |
|---|---|---|
| `event_file_name_*` (6) | adapter unit | naming contract: seq 0/1/2/42/999, colon/space substitution |
| `create_event_file_*` (5) | adapter unit | fresh dir; taken plain → `-001`; taken seq → next; exhaustion → `PortError`; non-collision I/O error not retried |
| `event_dispatch_same_id_multiple_events_same_consumer_gets_sequences` | use-case | the incident: 4 same-id events → seq 1,2,3,4, payloads in order, 4-entry log |
| `event_dispatch_two_consumer_knots_same_loom_get_sequences` | use-case | secondary path: 2 knots, 1 loom, 1 event → seq 1,2 |
| `event_dispatch_different_event_ids_same_loom_stay_seq_zero` | use-case | different directories stay plain-named |
| `fan_out_four_events_same_second_all_delivered_and_processed` | acceptance (`tests/event_fanout.rs`, real dispatcher) | 4 files, 4 names, 4 payloads, 4-entry `EventsDispatched`, consumer processes 4 strands, `extract_event_metadata` round-trips |
| `loom_log_read_all_skips_legacy_3tuple_events_dispatched` | adapter unit | graceful degradation of pre-0.33.0 log lines |
