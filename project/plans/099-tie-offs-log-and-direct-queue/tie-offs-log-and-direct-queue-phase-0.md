# Phase 0: Technical exploration and baseline

Plan 099 — Unified Tie-Off Log and Direct Event Queueing.

No behaviour change in this phase. This phase pins the code surface,
re-verifies the test table, and settles the open technical questions the
plan left for exploration.

## Baseline

- `cargo test` at branch head `a445278`: **green** (all unit + integration
  suites).
- Working tree carries 12 trivial skill-description trims (unrelated to
  this plan) — left alone.

## Pinned code surface

### Tie-off write path

- `ProcessStrand::compute_tie_off_path`
  (`src/application/usecases/process_strand.rs:608`) — derives
  `tie-offs/<rig>/<loom>/tie-off-<knot>.md` via
  `domain::knot_file::derive_tieoff_path`. **Static per (loom, knot)** —
  the strand path is already ignored. Phase 4 replaces the returned path
  with the single `tie-offs/<rig>/tie-offs.log`.
- `FileSystemTieOffSink::append`
  (`src/adapters/outbound/tieoff_sink.rs:52`) — builds the section:
  `## {knot} triggered by {event} {strand}` + `Timestamp:` + optional
  `session:` + `event:` / `source:` / `original_strand:` + `---` +
  content, joining sections with `\n---\n`. `TieOff` carries
  `knot_name`, `event_type`, `strand_path`, `timestamp`,
  `event_metadata`, `session_id`.
- `TieOffSink::read_content` — read by the Deleted-event history path
  (`extract_last_n`, last 5 sections for the same strand) and by the
  enforcement follow-up. Phase 4 must keep read working against the
  unified log.

### Event parse and dispatch

- `tieoff_parser::extract_agent_events` → `Vec<AgentEvent>`
  (`event_id`, `occurred`, `payload: HashMap`, `body: Option`).
- `ProcessStrand::dispatch_agent_events` (process_strand.rs:837) —
  parse → filter `occurred` → `dispatch_events_to_consumers` →
  `LoomEvent::EventsDispatched { dispatches, queue_busy }`.
- `dispatch_events_to_consumers` (process_strand.rs:762) — scans every
  loom's knots with `strand_source.resolve_for_producer(...)` and exact
  `event_id` matching; builds `DispatchRequest`s.
- `dispatch_grouped`
  (`src/application/usecases/system_event_emitter.rs:75`) — shared by
  agent-event and **system-event** (plan 082) dispatch. Groups by
  `(consumer_loom, event_id)` for filename sequencing and calls
  `EventDispatcherPort::dispatch` per request, returning
  `(event_id, consumer_knot, consumer_loom, created_path)`.
- `FileSystemEventDispatcher::dispatch`
  (`src/adapters/outbound/event_dispatcher.rs`) — writes
  `tie-offs/<rig>/<consumer-loom>/<EventId>/event-<ts>[-NNN].md`
  (frontmatter `event-id` / `target-knot` / `timestamp` / payload +
  markdown body). **This is the artefact the plan retires.**

### Queue

- `PendingEvent { id, kind, loom_id, knot_id, strand_path, queued_at }`
  (`src/domain/pending_event.rs:81`) — one JSON file per entry in
  `tie-offs/<rig>/events/{ts}-{hex}.json`. `kind` ∈
  `Created|Modified|Deleted`.
- `StrandEventQueue` trait (`src/application/ports.rs:911`) —
  `push` / `push_or_replace` / `pop` / `front` / `snapshot` / `delete` /
  `pending_event` / `push_shutdown` / `wait_for_item`.
- `StrandQueueAccessor` trait (`src/domain/events.rs`) —
  `pending_strand_paths` / `pending_entries` / `delete`.
- `DiskBackedEventQueue` (`src/adapters/outbound/disk_event_queue.rs`) —
  the persistent implementation; `load_persisted()` re-seeds the
  in-memory index from the directory at startup.
- Watcher path: `NotifyEventSource` → `DebounceEngine` (100 ms) →
  `PendingEvent` push. **Event delivery currently rides this path** — the
  dispatched `.md` is watched, debounced, and queued as a `Created`
  entry that *points at* the `.md`.
- Consumer: `ProcessStrand::execute_with_pending` → `TryInto<StrandEvent>`
  → `execute_inner(event, Some(id))`. Late removal at every terminal
  point; on success the removal is the last step before the commit.

### Consumer prompt / file dependency

- `resolve_config_and_build`
  (`src/application/usecases/process_strand_helpers.rs:206`) —
  `read_continuation_stamps(strand_path)` (front-matter budget stamps),
  `knot.prompt_template.instructions` as the base prompt,
  `AgentEventsContextProvider` listener context, then
  `session_resume::execute_with_resume(..., strand_file_ref: Option<StrandPath>, ...)`
  — `strand_file_ref` is what the adapters pass to pi as the `@file`
  the agent reads. **For a payload-embedded delivery there is no file:
  the payload must arrive in the prompt and `strand_file_ref` must be
  `None`.**
- `write_tie_off` → `extract_event_metadata(strand_path)` reads the
  dispatched file's front-matter to fill the tie-off's
  `event:` / `source:` / `original_strand:`. With no file, this must
  come from the delivery.

### Git commit

- `FileSystemGitVersioner::commit(loom, knot, strand, event_type,
  tie_off_content)` (`src/adapters/outbound/git_versioner.rs:214`) —
  `git add -A` → `unstage_rig()` → `subject = build_subject(...)` +
  `\n\n` + `truncate_body(content)` (1000-line cap) → `git commit -m`.
  **No trailers today.** Phase 3 appends the trailer block to the
  message (git's own trailer convention: last paragraph,
  `Token: value` lines).

### Service log

- `render_loom_event_line` (`src/adapters/service_log.rs:33`) — one
  `[KNOT][EVENT] <Variant> key=value…` line per `LoomEvent`.
  `EventsDispatched` renders `dispatch=` per dispatch. Phase 3 adds
  `session=` / `event=` / `source=` / `original-strand=` to the
  run-lifecycle lines.

### Plan 097 visibility surface

- `AgentEventsContextProvider`
  (`src/application/usecases/context_providers.rs`) +
  `build_pending_index` / `count_event_types` / `event_type_from_path`
  (`src/domain/pending_event.rs`) — derive the event type from the
  **dispatch directory name** and the producer from the dispatched
  file's `target-knot` front-matter. With no dispatched files, both
  must come from the queue entry's embedded delivery.

## Decisions on the open questions

1. **Queue-entry shape.** `PendingEvent` gains
   `delivery: Option<EventDelivery>` (`serde` default `None`, skipped
   when absent — existing on-disk entries load unchanged). A new
   `kind: "Event"` marks a delivered emission; the filesystem kinds
   keep `delivery: None` and stay pointers to real files, which is
   correct for them. `EventDelivery` carries `event_id`,
   `producer_knot`, `producer_loom`, `original_strand`, `body`, and
   the payload fields (`BTreeMap` for deterministic JSON).

2. **Synthetic strand descriptor.** An event-delivered entry's
   `strand_path` is `event:<producer-knot>/<event-id>` — not a file.
   It keeps `strand=` meaningful in every log line and tie-off header,
   and it is greppable back to the producer. File validation is skipped
   for the `Event` kind (there is nothing to stat).

3. **Consumer pipeline branch.** `StrandEvent` gains a fourth variant
   `EventDelivered { loom_id, knot_id, strand_path, delivery }`. Only
   four non-test match sites exist in `process_strand.rs` (the
   `execute_inner` destructure and the `validate_strand`
   deleted-check), so the variant is cheap and keeps the branch
   explicit rather than smuggling a payload through a fake path.

4. **Cutover: hard cutover of the write format, no config flag.** The
   service owns the write format (096's precedent is deprecate-and-
   warn for *keys*, not dual-write for *artefacts*). Old per-knot
   tie-off files and dispatch directories are **not** backfilled and
   **not** read by the new writer; they stay in git history. At startup
   the service detects retired-layout artefacts and logs a one-line
   `[KNOT][CONFIG]` deprecation diagnostic naming them.

5. **System events ride the same direct-queue delivery.**
   `dispatch_grouped` is shared with plan 082; making the queue the
   delivery mechanism for *all* event dispatch is what actually retires
   the file mechanism. A queue-only path for agent events with the file
   path kept for system events would keep the whole watcher dependency
   alive for no reason.

6. **Plan 097 visibility repoints to the entry.** `QueueEntryView` and
   `QueuedEventMeta` gain an explicit `event_id` and `producer` taken
   from the delivery; `build_pending_index` prefers them and keeps the
   path-derivation fallback so legacy dispatched files still index
   during the transition. The payload being *in* the queue does not
   change the index's metadata-only contract — the index still lists
   `(path, queued_at)` pairs only.

7. **Truncation contract unchanged.** The tie-off content stays the
   fidelity source and the 1000-line commit-body cap stays. Slim event
   audit blocks in `tie-offs.log` shrink the *log*, not the commit
   body; the full event bodies remain reachable via `pi --session-id`.

8. **Manual dispatch.** The "drop an `.md` in a dispatch directory"
   manual path retires with the files. The direct queue write is the
   sole manual mechanism and accepts the `Event` kind; `knot step
   --event <entry>` keeps working unchanged because it addresses queue
   entries by id.

## Re-verified test table

| Test | Covers | Baseline |
|---|---|---|
| `tests/consolidated_log.rs` | single-line `[KNOT][EVENT]`/`[KNOT][STATE]` grammar; `state.json` change-driven rewrites | ✅ green |
| `tests/event_fanout.rs` | fan-out to subscribers via dispatch files | ✅ green — **migration target** (pins the retiring behaviour) |
| `tests/event_enforcement.rs` | expected-vs-emitted enforcement + follow-up tie-off append | ✅ green — follow-up must re-target `tie-offs.log` |
| `tests/queued_events_e2e.rs` | queued-event visibility (plan 097) | ✅ green — repoints to entry-carried ids |
| `tests/persistent_queue.rs` | queue persistence, `load_persisted` redelivery | ✅ green — extended for `Event` kind |
| `tests/late_removal.rs` | at-least-once removal at every terminal point | ✅ green — extended for `Event` kind |
| `tests/agent_integration.rs` | end-to-end knot processing through pi | ✅ green |
| `tests/system_event_subscriptions.rs` | plan 082 system-event delivery | ✅ green — rides the new delivery in Phase 2 |
| `tests/tie_off.rs` | tie-off section format | ✅ green — re-targeted in Phase 4 |

## Test gaps to close (carried into the phase panes)

- Payload-embedded entry: per-subscriber fan-out with the payload in the
  entry body; removal at completion and at failure; `load_persisted`
  redelivery after a simulated crash; direct queue write of the `Event`
  kind. → Phase 1 pane.
- Absence assertions: no `event-*.md` written, no per-`EventId`
  directory created, no watcher registration needed for event delivery.
  → Phase 2 pane.
- Commit trailers: subject unchanged, trailer block present and
  parseable by `git interpret-trailers`. → Phase 3 pane.
- `tie-offs.log`: section grammar, sole-writer serialisation under
  concurrent appends, enforcement follow-up appends to the same file.
  → Phase 4 pane.
- Trace-chain acceptance: one emission traced service log → commit
  trailers → `tie-offs.log` with no persistent payload duplication.
  → Phase 5 pane.
