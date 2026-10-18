# Plan: Queue-Backed Pending-Event Visibility for Producers

## Related PRD

This plan extends [Persistent Events — Disk-Backed Event Queue](../../prds/prd-persistent-events.md).
The queue is that PRD's artifact; this plan adds a **producer-facing read view over
the queue** — the "what is already pending of each type I may emit?" index. It also
serves (without owning) the k2k-loop-control goals of
[prd-system-reliability](../../prd-system-reliability.md): duplicate suppression at
the source is the cheapest loop breaker.

## Problem

Plan 060 (v0.28.0) gave producer knots visibility of pending events: a
`## Pending Events` section in the prompt listing event files that "have been
dispatched but are still present", with the norm *"check these before deciding to
emit — if a pending event covers the same outcome, you may not need to emit a
duplicate."* The design intent is right; the **source of truth it keys on is stale**.

Since the persistent queue (plan 065) and the runtime-tree split (plan 068), the
authoritative set of *pending* events is the queue (`tie-offs/<rig>/events/*.json`).
Dispatch directories accumulate forever — files are not removed on consumption — so
"still present on disk" is not a signal of pending. As of drafting (verified
against the working tree at drafting — `c63608c` plus the in-flight uncommitted
096-series changes), the 060 index is re-keyed to the queue *when a queue
reference is wired* (`AgentEventsContextProvider::scan_pending_events` →
`StrandQueueAccessor::pending_strand_paths`), but the re-key is partial and the
advisory is soft. A field instance (2026-10-18, recipt-parser rig, Knot 0.60-class
service) shows the failure mode: byte-identical `PlanRequested` events and
near-duplicate `PhaseReady` / `PlanScoped` events each triggered a full agent
session, fanning out through four consumer looms — visibility (however keyed)
plus a soft norm did not suppress the duplicates.

Remaining gaps (verified at drafting, against the working tree):

1. **No cap.** A backlog of N queued files becomes N unbounded entries in the
   prompt — the read-bloat anti-pattern the multi-session design exists to prevent.
2. **Not metadata-only.** Every matching queued file is read for frontmatter
   (producer filter via `target-knot`, plus `description` and `timestamp` carried
   into the index). Queue entries store the *target* (consumer) loom/knot, not the
   producer, so the producer filter cannot yet come from the queue alone (see
   Target 1).
3. **Directory-scan fallback.** With no queue reference wired the provider falls
   back to `scan_dispatch_directory` — the stale-signal path survives.
4. **Stale-by-construction timing.** The index is computed at prompt build; a long
   session can outdate it, and nothing is logged when the queue grows underneath a
   running session.
5. **Advisory with no backstop.** Correctness rides entirely on producer
   discretion. That is acceptable (batching was rejected as the over-correction —
   see Notes), but the advisory needs a small *enforced* floor for the
   identical-event case.

## Target

1. **Queue is the pending set.** The producer's pending-event index is built from
   the persistent queue, not the dispatch directory: per event type, the **count**
   of queued entries plus the **newest N paths and `queued_at`** (N capped, e.g.
   10). Scoped to the event types this knot may emit (the runtime already computes
   that list for the emission instructions). **Metadata-only (body-blind)**: the
   event type is derived from the dispatch path embedded in the queue entry and
   `queued_at` comes from the queue entry — no event bodies are parsed. Producer
   identity still requires a file read (queue entries store the *target*, not the
   producer — verified in `domain/pending_event.rs`): the default is a
   frontmatter-only `target-knot` read (metadata, no body); a producer field on
   the queue entry is the alternative (Phase 1 decision). The section is omitted
   entirely when the queue holds nothing of the knot's emittable types.
2. **The norm survives, and is auditable.** The dedup norm ("if a queued event
   already carries the same facts — compare the frontmatter keys such as
   plan/phase — emit `event: None` instead of queueing a second signal") remains in
   the injected context, and the wording moves to the per-rig instruction surface
   (the `knot-template` skill, plans 095/096) so it is visible and versioned with
   the rig, not buried in a prompt builder.
3. **Passive growth observation.** When a knot's tie-off dispatches an event and
   the queue *gained* same-type entries for that knot during the session, the
   `EventsDispatched` log line carries a note (e.g. `queue-busy=<n>`) — no extra
   agent turn, no bounce-back. This is the observability that decides whether a
   parse-time re-ask (explicitly out of scope here) is ever warranted.
4. **Gated increment — enforced identical-event backstop.** Only if the field
   evidence after (1)–(3) still shows duplicate *invocations*: drop a queued entry
   whose strand file is hash-identical to one already processed for that
   `(loom, knot, event-type)` in the current drain, with a logged
   `EventCoalesced` record. Per-item queue semantics (at-least-once, fault
   isolation) are preserved — this dedups *before* invocation, never inside a
   session.

## Existing Tests

Verified against the working tree at drafting (`c63608c` + the in-flight
096-series changes; Phase 0 re-verifies and records deltas):

| Test class / file | What it covers | Status |
|---|---|---|
| `application/usecases/context_providers.rs` — `AgentEventsContextProvider` tests | Queue-preferred pending scan (`StrandQueueAccessor`), dispatch-dir scan fallback, section formatting, omit-when-empty | ✅ Green — **but the index carries `description`/`timestamp` from per-file frontmatter reads with no cap; the definition under test is what this plan replaces** |
| `domain/events.rs` — `build_listener_context*` tests | Emission-instruction generation (incl. the `extract_emitted_event_ids` scoping list), event dedup across consumers, loom-level subscriptions | ✅ Green |
| `adapters/outbound/event_dispatcher.rs` tests | Dispatch file writing, frontmatter structure (`target-knot` = producer), fan-out | ✅ Green |
| `adapters/outbound/disk_event_queue.rs` tests + `tests/persistent_queue.rs` | Disk queue push/pop/restart-scan/late-removal, `pending_strand_paths`, `tie-offs/<rig>/events/*.json` shape | ✅ Green (per 065) |
| `application/usecases/process_strand.rs` execution tests + `tests/pipeline.rs` | Full pipeline incl. context injection and `EventsDispatched` emission | ✅ Green |
| `adapters/service_log.rs` — `EventsDispatched` render test | `EventsDispatched` log-line format | ✅ Green |

## Test Gaps

- No test defines "pending" in terms of **queue entries** (vs. directory files).
- No cap/backlog test: 50 queued entries must not become 50 prompt entries.
- No test that the index is **scoped to the knot's emittable types** only.
- No metadata-only test: the index must be buildable from queue metadata +
  path alone (producer filter from frontmatter only — no `description` /
  `timestamp` carry-over, no body reads).
- No `queue-busy` observation test on `EventsDispatched` when the queue grew
  mid-session.
- No integration test: producer re-entry with a same-type event **queued** → the
  queued entry (not a directory file) appears in the prompt.
- (Gated increment) No `EventCoalesced` hash-dedup test.

## Phases

### Phase 0: Verify the current pending-set wiring

Read the tree, record in the phase doc what the current `AgentEventsContextProvider`
actually keys on (dispatch-dir scan vs. queue), whether consumed files are ever
removed from dispatch dirs, and what the queue entry exposes (fields of
`events/*.json`). **Pane:** existing suite green + the recorded findings
(deviations recorded, not silently fixed). File set: none — this phase commits
findings only.

### Phase 1: Queue-backed pending set (domain/application)

Build the pending-set source of truth: a function/usecase that, given a knot's
emittable event types, reads the persistent queue and returns per-type
`(count, newest-N {path, queued_at})`. Cap constant fixed. Content-blind by
Metadata-only by construction (type from path, `queued_at` from the entry; the
producer-identity source is decided here — frontmatter-only read vs. a producer
field on the queue entry). **Pane:** the targeted unit test suite for the
pending-set builder (includes cap, scoping, empty, metadata-only cases).

### Phase 2: Prompt injection of the Queued Events section

Replace/augment the 060 directory scan in the context provider with the queue-backed
index; reword the section ("queued, awaiting consumer invocation") and the dedup
norm. Omit-when-empty retained. Pure behaviour change in the prompt only; emission
and enforcement flows untouched. **Pane:** the context-provider prompt-content test
suite green (old directory-scan tests updated to the new source).

### Phase 3: Dedup norm on the per-rig instruction surface

Move the norm wording into the `knot-template` instruction content so producer
rigs carry it visibly and it is versioned with the rig (per 095/096). The injected
prompt references the rig's wording. **Pane:** prompt-template + instruction-surface
conformance tests green.

### Phase 4: `queue-busy` observation on dispatch

At tie-off dispatch, compare the queue count for the dispatched type before/after
the session; if it grew, append `queue-busy=<n>` to the `EventsDispatched` log
line. **Pane:** the dispatcher log-line test.

### Phase 5: End-to-end verification

Full-pipeline integration: producer emits → entry lands in queue → producer
re-enters (same strand, Modified) → the **queued** entry appears in the prompt with
the norm; mock runner shows the re-entry emitting `event: None` when the queued
event covers the same facts. **Pane:** the integration test green.

### Phase 6 (gated increment): `EventCoalesced` hash-dedup backstop

**Enter only if** post-release field evidence (the `queue-busy` lines + service
logs from a live rig) still shows duplicate *invocations* on identical queued
events. Before the next invocation, drop a queued entry whose strand file hash
matches one already processed for that `(loom, knot, event-type)` in the current
drain; log `EventCoalesced` with the dropped path and the retained path. Queue
delivery semantics (at-least-once, per-item) unchanged. **Pane:** the coalescing
test suite (identical dropped, distinct kept, different types never compared).

## Notes

- **Extends, does not supersede, 060.** The 060 architecture (first-class
  `ContextProvider`, omit-when-empty, producer-side advisory) is kept; only the
  pending-set source and the norm's home change. If Phase 0 finds the tree has
  already moved on from the 060 design, re-record the delta there.
- **Drafting-time verification.** The wiring facts above were verified against
  the working tree at drafting (2026-10-18 — `c63608c` plus the in-flight
  uncommitted 096-series changes): the queue-preferred scan already exists, the
  frontmatter reads are current, and the queue entry carries no producer field.
- **In-flight blind spot.** `pending_strand_paths()` covers debounced queue
  entries only; popped (in-flight) entries are invisible, and their disk file is
  removed at pop. If post-release field evidence shows the dominant duplicate
  pattern is in-flight overlap (a producer re-entering while its own event is
  mid-processing), a later increment could union the `state.json` strand queue
  into the index. Out of scope here.
- **Batching was considered and rejected** for this problem: it trades per-item
  fault isolation and the 1:1 strand→queue→completion mapping for a win that
  dedup + visibility already deliver. Revisit only with evidence that distinct
  (not duplicate) items dominate the queue cost.
- **Advisory, not enforced — by design.** (1)–(3) reduce duplicates; they do not
  guarantee it. Phase 6 is the enforced floor, and only for the identical case.
  The same-facts-but-different-text case remains the agent's judgment.
- **Producer/consumer asymmetry is the one deliberate leak.** Producers are not
  told their consumers' wiring; the engine mediates this by exposing *its own*
  queue state, scoped to the knot's emittable types. Do not widen to arbitrary
  queue inspection.
- **Prompt-surface migration.** Producer prompts change when the queue is
  non-empty — note for the `knot-update` skill and the release notes; existing
  rigs can ignore the section (additive).
- Field evidence (non-normative): 2026-10-18 recipt-parser duplicate chain —
  `PhaseReady(5b)` ×2 → `ImplementationNote(Phase 6)` ×2 → byte-identical
  `PlanRequested` ×2 → `PlanScoped` ×2 queued for `plan-author`; each duplicate
  cost a full agent session.
