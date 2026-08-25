# Plan: Queue Entry Identity Self-Heal — Filename Is the Event ID

## Related PRD

This plan contributes to the persistent-event-queue work defined in
[065 Persistent Event Queue](../065-persistent-event-queue/persistent-event-queue-plan.md)
("the disk **is** the queue") and the late-removal / front-based loop
introduced by [73 Knot Step](../73-knot-step/knot-step-plan.md). Plan
065 documented the invariant *"the file ID matches the
`PendingEvent.id` field"* but nothing enforces or repairs it. This plan
makes that invariant self-healing so a single externally-touched queue
file can never wedge the pipeline.

## Problem

On 2026-08-25 a running rig (borrow-my-stuff) went **idle twice while
the queue was non-empty** — `state.json` and `tie-offs/<rig>/events/`
showed queued events, the service was alive and writing fresh state,
but no event was ever picked up. Both stalls required a process restart
to clear; the second required additionally deleting an orphaned queue
file by hand.

### Mechanism (verified against code and the rig's event directory)

Queue entries are `tie-offs/<rig>/events/{id}.json`. FIFO order comes
from **filename sort**, but every file operation resolves the path from
the **JSON `id` field**:

- `FileSystemEventStore::scan_events()` sorts by filename, returns the
  parsed events (carrying the JSON `id`) —
  `src/adapters/outbound/event_store.rs`
- `DiskBackedEventQueue::front()` → `read_event(&first.id)` — resolves
  by JSON id, and **swallows a read failure with `.ok()`**, returning
  `None` — `src/adapters/outbound/disk_event_queue.rs`
- `pop()` — same resolution, but panics on failure (`.expect`)
- `delete()` / late removal in `ProcessStrand::execute_with_pending` —
  removes `{id}.json` by JSON id
- `push_or_replace` dedup — removes existing files by JSON id

The operator backdated a manually created rectify event by **renaming
its queue file** (14:52 name → 14:04 name) to front the FIFO. The JSON
content still carried the original id (`1787633553354-…`). From that
moment the queue held a file with **filename ≠ JSON id**, and:

1. **Phantom head → permanent idle (incident #1, 15:05:29).** After the
   previous event finished, the consumer loop peeks
   `front()`. The scan finds the backdated-named file first;
   `read_event(original id)` hits a file that no longer exists; the
   error is swallowed; `front()` returns `None`. The loop
   (`next_event` in `spawn_process_strand_loop`, `src/server.rs`)
   interprets `None` as *queue empty* and blocks on `notified()`.
   Pushes made during processing fired `notify_one()` with no waiter
   registered (tokio `Notify` is non-persistent — see plan 076), so
   nothing ever woke the loop. `snapshot()` (which state.json uses)
   counts files, so state kept showing a non-empty queue. Silent,
   forever, no log line anywhere.
2. **Restart propagated the divergence (15:22).**
   `load_persisted` → `push_or_replace` → dedup removal by JSON id is a
   no-op on the renamed file, and `write_event` writes under the JSON
   id — the same logical event now existed **twice** (renamed file +
   restored original-id file, same dedup key).
3. **Late removal created a new orphan (incident #2, 15:30:43).** The
   run processed the backdated-named head (reading the restored
   original-id file's content) and late-removed by JSON id — deleting
   the *restored* file and leaving the *renamed* file orphaned with a
   dangling id. After the successful run dispatched six new events, the
   consumer peeked again: the orphan sorts first (14:04), its id's file
   is gone, `front()` returns `None` — the same permanent idle, now
   with seven queued events.

### Why this is a Knot bug, not an operator error

- The queue is documented (knot-dispatch skill, plan 065) as directly
  observable and writable files — "the disk is the queue". Renaming a
  queue file to reorder FIFO is a natural operator action, and
  `front_reflects_on_disk_edits` even blesses on-disk content edits.
- The invariant break was **silent**: `front()` logs nothing when the
  head is unreadable, so the only visible symptom was "rig alive,
  queue non-empty, nothing happens" — which reads as a runtime bug.
- The queue's own bookkeeping (dedup, late removal, `load_persisted`)
  could not repair the divergence; it **amplified** it (duplicate file,
   then orphan). One externally touched file wedged the entire rig
   indefinitely.

## Target

When this plan is complete:

1. **The filename stem is the queue entry's identity.** On every scan,
   if a file's JSON `id` differs from its filename stem, the queue
   self-heals: the JSON `id` is rewritten (atomic temp→rename) to the
   stem and a warning is logged. The filename (and therefore the FIFO
   position) is authoritative — an operator's backdate/rename keeps its
   intended order, and `queued_at` is preserved as the honest record.
   After the first scan touching the file, the invariant
   (`filename stem == JSON id`) holds and no further rewrites occur.

2. **A renamed file can never wedge the pipeline.** `front()` and
   `pop()` return the head event (with the normalised id) instead of
   `None` when the sole problem is name/id divergence. Dedup
   removal, late removal, `load_persisted`, `knot step --event`
   resolution, and state.json snapshots all operate on the healed id —
   they remove/heal the *actual* file instead of a dangling one.

3. **A vanished head is graceful and visible.** If the head file
   disappears between scan and read (concurrent late-removal or
   `knot step`), `front()` returns `None` for that cycle **and logs a
   warning naming the file**; `pop()` rescans once and retries instead
   of panicking (replacing the current `.expect`).

4. **Duplicate-key collapse on load.** `load_persisted` over a queue
   containing two files with the same dedup key (the exact post-incident
   state) collapses them to one entry via the existing
   `push_or_replace` semantics (latest position wins), leaving no
   orphan behind.

5. **The operator action is documented.** The knot-dispatch skill and
   `docs/concepts.md` state that renaming a queue file reorders the
   FIFO (the queue repairs the internal id on next scan) — the
   borrow-my-stuff monitoring workflow becomes a supported operation.

## Non-Goals

- Changing FIFO semantics (filename sort stays the order; `queued_at`
  stays metadata, not ordering).
- Watcher reliability (missed file events) — a different failure class;
  the events in the incident were correctly queued.
- Malformed-JSON queue files (skipped with a warning — unchanged) and
  `.json.tmp` crash leftovers (skipped — unchanged).
- The lost-`Notify`-permit race in the consumer loop — plan 076.

## Existing Tests

| Test | What it covers | Status |
|------|----------------|--------|
| `disk_event_queue.rs` unit tests (`front_*`, `pop_*`, `push_or_replace_*`, `load_persisted*` via `events_survive_on_disk_across_queue_recreation`, `*_on_disk_modification`) | front/pop/snapshot/dedup semantics incl. **content** edits on disk (`front_reflects_on_disk_edits`) | ✅ Green — content edits blessed, **name** edits untested |
| `event_store.rs` unit tests | atomic write, scan sort/skip-malformed, read by id | ✅ Green |
| `tests/late_removal.rs` Part B (`front_loop_drains_queue_then_idles`, crash-window, poison-pill) | Full composition: real queue + front-based service loop + mock `pi`; drain → `QueueIdle` | ✅ Green — the template for the incident-repro test |
| `tests/persistent_queue.rs` | Restart survival, `load_persisted` ordering | ✅ Green |
| `tests/step.rs` | `knot step` head/`--event` resolution | ✅ Green |

## Test Gaps

- No test renames a queue file (filename ≠ JSON id) — the incident
  state. Current behaviour in that state: `front()` → `None` (silent
  wedge), `pop()` → panic, dedup/late-removal → wrong/no file removed.
- No test reproduces the incident end-to-end (backdated head + queued
  tail → pipeline must process the head first and drain, not idle).
- No test for `load_persisted` over duplicate-dedup-key files.
- No test that late removal deletes the file that was actually peeked
  when the file was renamed mid-queue.
- No observable log line for any of the above failure paths.

## Phases

### Phase 1: Identity normalisation in the event store (TDD)

`src/adapters/outbound/event_store.rs` + `src/adapters/outbound/disk_event_queue.rs`.

1. `scan_events()` normalises each entry: if the parsed `event.id.0`
   differs from the filename stem, repair on disk — atomically rewrite
   the file (existing temp→rename path) with `id := stem` — and log one
   stderr warning per repaired file:
   `[queue] repaired event file {name}: id {old} -> {stem} (filename
   is the queue identity)`. Return the normalised events. A missing or
   empty `id` is still a parse failure (malformed — existing skip +
   warning path, message extended to hint at the name/id rule).
2. `front()`: unchanged shape (scan → read head), which is now correct
   because the scan heals the id first. If `read_event` still fails
   (file vanished between scan and read — concurrent removal), log
   `[queue] head {name} vanished before read (concurrent removal?)`
   and return `None` — never panic, never silent.
3. `pop()`: replace the `.expect("failed to read event file on pop…")`
   with one rescan-and-retry; if the same head is still unreadable, log
   the warning from (2) and return `None` (graceful, no panic).
4. `push_or_replace` / `load_persisted` / `delete` /
   `StrandQueueAccessor::delete` — no signature changes; correctness
   follows from the healed invariant (removal by id now targets the
   real file). Verify by test, not by change.

Unit tests (failing first, then green) in `disk_event_queue.rs` /
`event_store.rs`:

- `scan_repairs_renamed_file_and_normalises_id` — push event (id A),
  rename file to B on disk; `scan_events` returns the event with
  `id == B`; the file on disk is rewritten with `id == B`; a second
  scan performs no rewrite (mtime/content stable).
- `front_returns_renamed_head_without_wedging` — the incident state:
  two files, head renamed to an earlier timestamp; `front()` returns
  the head event (id healed), `len()` unchanged, no `None`.
- `pop_reads_renamed_head_without_panicking` — same setup through
  `pop()`; event returned, file removed, tail intact.
- `front_vanished_head_is_graceful_none` — push, delete the file by
  hand behind the queue's back, `front()` → `None` (no panic); a
  subsequent push is visible again.
- `pop_vanished_head_rescans_and_returns_tail` — head deleted by hand;
  `pop()` returns the tail event.
- `dedup_removes_renamed_file_by_healed_id` — head renamed;
  `push_or_replace` with the same dedup key removes the renamed file
  (no dangling duplicate).
- `late_removal_deletes_the_peeked_file` — push, peek via `front()`,
  rename the file, then `StrandQueueAccessor::delete(&event.id)` —
  the actually-peeked file is gone, no orphan remains. (This is the
  exact incident-#2 orphan path; with normalisation the peeked id
  already equals the stem, so `delete` hits the right file — the test
  pins it.)

### Phase 2: Incident-reproduction integration tests (full composition)

New `tests/queue_identity.rs` (Part-B style, mirroring
`tests/late_removal.rs`: real `DiskBackedEventQueue`, real
`spawn_process_strand_loop`, mock `pi` script, `wait_until` helper):

1. `backdated_head_processes_first_and_drains` — **the incident
   repro.** Pre-seed the queue with two events (E1 tail, E2 head) via
   direct file writes (the established watcher-free pre-seeding
   technique, plan 73 phase 4). Rename E2's file to an earlier
   `{timestamp}-{hex}.json` name while leaving the JSON id untouched
   (the 14:52 backdate). Start the pipeline. Assert, in order:
   E2 is processed **first** (loom-log `KnotProcessing`/`KnotCompleted`
   + git commit), E2's queue file is removed with **no orphan left**,
   E1 is then processed, the events dir is empty, and the rig-log
   eventually records `QueueIdle` (i.e. the loop drained and went idle
   *cleanly* — the pre-fix behaviour is `QueueIdle` with the queue
   still full).
2. `restart_over_duplicate_key_files_collapses_to_one` — pre-seed the
   post-incident state: a renamed file plus a file carrying the same
   JSON id and same dedup key. Start the pipeline (which runs
   `load_persisted`). Assert the queue collapses to one event for that
   key, it is processed once, and no orphan file remains.
3. `step_mode_renamed_head_does_not_panic` — `step_knot` head path
   (`step_head_event`) over a renamed head: the event is processed,
   exit 0, file removed. (Pins the `pop`/`front` panic path fixed in
   Phase 1.)

Full `cargo test --no-fail-fast` green; in particular
`tests/late_removal.rs`, `tests/persistent_queue.rs`, and
`tests/step.rs` must stay green unchanged.

### Phase 3: Documentation, skills, changelog

- `.agents/skills/knot-dispatch/SKILL.md` — extend the "Event Queue"
  section: renaming a queued event's file reorders the FIFO; on the
  next scan the queue repairs the file's internal id (filename is the
   queue identity, `queued_at` is preserved). State that this is the
  supported way to front a queued event (e.g. a manual rectify).
- `docs/concepts.md` — queue paragraph: same name/id rule in one
  sentence (the skill stays the operator-facing detail; concepts.md
  carries the invariant).
- `.agents/skills/knot-update/SKILL.md` — changelog entry for the
  bumped version: no document migration; queues containing renamed or
  hand-edited event files self-heal on the first scan of the new
  binary (warning logged per repaired file).
- Version bump (MINOR, `Cargo.toml` 0.36.0 → 0.37.0) and
  `docs/release-notes.md` entry are part of plan completion via the
  project-plan-completion skill.
- Publish updated skills globally per AGENTS.md (copy to
  `~/.agents/skills/` / `~/.agents/skills-library/`, verify with diff).

## Notes

- **Alternative considered and rejected — heal by renaming the file to
  match the JSON id** (keep the id, move the file). This would
  *undo* an operator's backdate (the event jumps back to its original
  position — the exact frustration that produced the manual rename in
  the first incident) and changes FIFO order silently. Filename-wins
  preserves the operator's ordering intent and matches "the disk is
  the queue": the file on disk *is* the entry, and its name *is* its
  position.
- **Alternative considered and rejected — resolve everything by
  scanned path (thread paths through front/pop/delete/late-removal).**
  Correct, but ripples through `StrandQueueAccessor`,
  `ProcessStrand`, and step-mode resolution. Normalising at the single
  choke point (scan) restores the existing id-based invariants
  everywhere with no signature changes.
- **Concurrency:** two scans racing on the same divergent file both
  perform the identical atomic rewrite — last rename wins, content
  identical, no corruption. The repair is idempotent (after one scan,
  name == id, no further rewrites).
- **Why a stderr warning and not a rig-log event:** the queue adapter
  has no rig-log port (it is constructed before the logging pipeline
  is wired for it); stderr (the service log) matches the queue's
  existing `[WARN] skipping malformed event file` style. Open question
  for a future plan: a durable `QueueEntryRepaired` rig-log event —
  the rig-log is cleared at startup (plan 072), so a repair that
  survives across restarts would benefit from the loom-log/tie-off
  record instead; decide then.
- **Relation to plan 076:** even with self-heal, the consumer loop's
  check-then-wait ordering can still lose a `Notify` permit in a
  microsecond window (same symptom class: idle with non-empty queue,
  no log). Plan 076 closes that race. The two plans are independent;
  075 is the observed-incident fix and should land first.
- **Incident forensics** (borrow-my-stuff, 2026-08-25): see
  `_monitor_notes.md` in that repo — timeline 14:52 (backdate rename)
  → 15:05:29 (idle #1) → 15:22 (restart; `load_persisted` duplicated)
  → 15:30:42 (orphan created by late removal) → 15:30:43 (idle #2)
  → 15:47 (orphan deleted by hand; restart #2). The monitor's
  14:53 note that "the running service processes the in-memory queue
  in arrival order" is a misdiagnosis of the phantom-head symptom —
  the queue is disk-based; the backdate appeared not to reorder
  because `front()` returned `None` for the renamed head.

## Implementation Status: ⬜ Not Started
