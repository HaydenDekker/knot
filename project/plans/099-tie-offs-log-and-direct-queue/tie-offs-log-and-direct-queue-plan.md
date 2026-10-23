# Plan: Unified Tie-Off Log and Direct Event Queueing

## Problem

The rig's runtime tree carries the same event information in redundant,
unbounded, and mis-typed places, and positions agents to read accumulating
logs as if they were state:

- **Per-knot tie-off files** (`tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md`)
  are append-only, grow forever, and are the artefact an agent is most
  likely to whole-file read "to see what the knot did" — an unbounded
  accumulation with snapshot readers.
- **Event payloads are duplicated**: the producer's tie-off event blocks
  *and* per-emission dispatched `.md` files (one per subscriber) both carry
  the payload, persistently and permanently.
- **Dispatch files do delivery work a record doesn't need**: the watcher
  turns a dropped md into a queue entry whose JSON is a *pointer* to the
  md — the persistent file holds the payload while the queue entry, which
  the convention says *holds the message*, carries only a path.
- **Trigger metadata** (strand path, event type, source knot,
  original_strand, session) sits in per-section tie-off headers where its
  readers (human, via developer debug) have better homes: the service log
  (live) and the git commit (durable, atomic with the work).
- **The on-disk accumulation duplicates git**: the commit body already
  carries the tie-off content, so the permanent history has a second,
  slower-to-query home.

The settled direction (worked through in the rig review):

| Layer | Carrier | Reader |
|---|---|---|
| What was delivered (prose + event audit) | one accumulating **`tie-offs/<rig>/tie-offs.log`** + the commit body (git is the rotation) | human, agent (query: grep/git log — never whole-file read) |
| Event delivery (payload) | queue entry with **embedded payload**, per subscriber, removed at completion | consumer agent, at delivery — never by reading the producer's log |
| Info trace (trigger metadata) | structured **service-log line** (live) + **commit trailers** (durable); `session:` is the user-debug link to `pi --session-id` | human via agent (developer debug) |

`occurred: false` acknowledgements have no delivery to make — they stay in
`tie-offs.log` only.

## Target

- `tie-offs/<rig>/` holds exactly: `state.json`, `knot-service.log`
  (user-redirected stdout, unchanged in nature), **`tie-offs.log`** (one
  accumulating log, service is the sole writer, appends serialised), and
  `events/` (transient queue). No per-knot files, no per-EventId
  directories.
- Events parsed from a producer's tie-off are queued **directly** as
  payload-bearing entries, one per subscriber; the dispatched `.md` files
  and the `event:`-URI strand/watch mechanism for events retire.
- Every knot commit carries **trailers** (`session:`, `event:`, `source:`,
  `original_strand:`) so the durable trace is atomic with the work;
  service-log lines carry the same fields structured for grep.
- The debug trace chain closes with no payload stored redundantly anywhere:
  service log (live, incl. failures) → git commit/trailers (durable) →
  `tie-offs.log` section (delivery record + audit) → `pi --session-id`
  (full agent narrative, incl. full event bodies).

## Existing Tests

Re-verified in Phase 0 (baseline `a445278` green):

| Test | What it covers | Status |
|------|---------------|--------|
| `tests/consolidated_log.rs` | Binary-level: single-line `[KNOT][EVENT]`/`[KNOT][STATE]` stderr grammar; `state.json` change-driven rewrites (plan 083) | ✅ Green — extended in Phases 3–4 for the new trace fields |
| `tests/event_fanout.rs` | Event fan-out to subscribers (the dispatched-file shape) | ✅ Green — pins the retiring behaviour; the migration target (the file path retires in Phase 5) |
| `tests/event_enforcement.rs` | Per-event enforcement / follow-up tie-off append (plans 091/094) | ✅ Green — the follow-up append re-targets `tie-offs.log` in Phase 4 |
| `tests/queued_events_e2e.rs` | Queued-event visibility (plan 097) | ✅ Green — repoints to the entry-carried ids in Phase 2 |
| `tests/persistent_queue.rs` | Queue persistence, `load_persisted` redelivery | ✅ Green — extended for the `Event` kind in Phase 1 |
| `tests/late_removal.rs` | At-least-once removal at every terminal point | ✅ Green — extended for the `Event` kind in Phase 1 |
| `tests/agent_integration.rs` | End-to-end knot processing through pi | ✅ Green — the e2e pane for the queue + log changes |
| `tests/system_event_subscriptions.rs` | System-event delivery (plan 082) | ✅ Green — rides the new delivery in Phase 2 |
| `tests/tie_off.rs` | Tie-off section format | ✅ Green — re-targeted to the unified log in Phase 4 |

## Test Gaps

- No test for a payload-embedded queue entry: fan-out per subscriber with
  the payload in the entry body, removal at completion, `load_persisted`
  redelivery after crash, and direct queue write of the new event kind.
- No test asserting the dispatched-md path is **absent** (no `events/`
  strand dirs created for event dispatch; no watcher dependency for events).
- No test for commit trailers (subject format is covered; the trailer
  fields are new).
- No test for `tie-offs.log`: sole-writer append serialisation under
  concurrent knots, section grammar (structured header line), and the
  enforcement follow-up section appending to the same file.
- No acceptance-level test for the trace chain: one emission traced through
  service log → commit → `tie-offs.log` → (session) with no persistent
  payload duplication.

## Phases

The file sets are firmed against the Phase 0 pinned surface; the Phase 0
decisions (D1–D8, recorded in the Phase 0 document) drive them.

### Phase 0: Technical exploration and baseline

Pin the code surface (tie-off append path, event parser, queue adapter,
git commit path), re-verify the test table, and settle the open technical
questions (see Notes). **Pane:** the baseline suite green + the recorded
findings (deviations recorded, not silently fixed). **File set:** the
phase document only — no behaviour change.

### Phase 1: Payload-embedded queue entries

The queue entry for a delivered emission becomes the message itself
(D1–D3): `PendingEvent` gains `delivery: Option<EventDelivery>` and a
new `kind: "Event"` (existing on-disk entries load unchanged — serde
default, skipped when absent); the entry's `strand_path` is the
synthetic `event:<producer-knot>/<event-id>` descriptor, never stat-ed;
the `StrandEvent` pipeline gains an explicit `EventDelivered` branch —
file validation skipped, no `@file` ref to the adapter, the rendered
delivery prepended to the prompt, batch stamps a fresh hop — and
`write_tie_off` takes its trace fields from the delivery rather than a
dispatched file. **Pane:** `tests/payload_queue.rs` green (per-
subscriber fan-out with the payload in the entry body, no dispatched
`.md` written, removal at completion and at failure, `load_persisted`
redelivery after a crash, prompt carries the payload with no `@file`,
legacy JSON loads). **File set:** `src/domain/pending_event.rs`,
`src/domain/events.rs`, `src/application/usecases/process_strand.rs`,
`src/application/usecases/process_strand_helpers.rs`,
`tests/payload_queue.rs`.

### Phase 2: Direct queueing from the tie-off parse

Events parsed from a producer's tie-off queue directly — one
payload-bearing entry per subscriber (D5: system events ride the same
delivery). The `QueueEventDispatcher` adapter is wired as the sole
`EventDispatcherPort` for both the agent-event and system-event paths,
sharing one persistent queue; the `event:`-URI strand/watch path
retires (no dispatch directory, no watcher, no `DirectoryCreated`);
the pending-event visibility index repoints to the entry-carried ids
(D6); the port contract carries `original_strand` and the producer's
loom into the delivery. **Pane:** the e2e absence assertions — producer
tie-off → consumer runs from the queue entry with the payload in its
prompt, and no dispatch `.md`, per-`EventId` directory, or watcher
exists. **File set:** `src/adapters/outbound/queue_event_dispatcher.rs`
(new), `src/application/ports.rs`,
`src/application/usecases/system_event_emitter.rs`,
`src/application/usecases/process_strand.rs`,
`src/application/usecases/loom/mod_watchers.rs`,
`src/application/usecases/context_providers.rs`,
`src/adapters/outbound/disk_event_queue.rs`,
`src/application/in_memory_event_queue.rs`, `src/domain/events.rs`,
`src/domain/pending_event.rs`, `src/server.rs`,
`src/application/usecases/config_event_handler.rs`, `tests/step.rs`,
`tests/system_event_subscriptions.rs`.

### Phase 3: Commit trailers and structured trace fields

Knot commits carry the trailer block (`session:`, `event:`, `source:`,
`original_strand:` — git's own trailer convention: last paragraph,
`Token: value` lines); the service-log line grammar gains the same
fields on the run-lifecycle lines, structured for grep. Fields that are
`None` (e.g. `original_strand` for a loom- or rig-scoped system event)
are omitted, not written empty. **Pane:** the commit-trailer tests
(subject unchanged, block present and parseable by `git
interpret-trailers`) + the consolidated-log grammar tests green.
**File set:** `src/adapters/outbound/git_versioner.rs`,
`src/adapters/service_log.rs`, `tests/consolidated_log.rs`, the trailer
tests (new, or extended in `tests/`).

### Phase 4: The unified `tie-offs.log`

The service appends every tie-off section to one
`tie-offs/<rig>/tie-offs.log` — sole writer, serialised appends — with
the trigger header reworked into a structured first line and the event
audit blocks slimmed to id, occurred, reason, and timestamp (no payload
bodies — the commit stays the fidelity source per D7; the full session
narrative stays reachable via `pi --session-id`). Per-knot files
retire; `TieOffSink::read_content` (the Deleted-event history path and
the enforcement follow-up) re-targets the unified log; the enforcement
follow-up append lands in the same file. **Pane:** the consolidated-log
binary-level test pane green (section grammar, sole-writer
serialisation under concurrent appends, enforcement follow-up appends
to the same file). **File set:**
`src/adapters/outbound/tieoff_sink.rs`, `src/application/ports.rs`,
`src/application/usecases/process_strand.rs`,
`tests/consolidated_log.rs`, `tests/event_enforcement.rs`,
`tests/tie_off.rs`.

### Phase 5: Cutover and migration

One planned change, never piecemeal (D4: hard cutover, no config flag).
Startup detects retired-layout artefacts (per-knot tie-off files,
dispatch directories) and logs one-line `[KNOT][CONFIG]` deprecation
diagnostics naming them; old files stay in git history, no backfill —
the service owns the write format. The retired file-dispatcher path
deprecates with diagnostics; the direct queue write becomes the sole
manual dispatch mechanism (D8) and accepts the `Event` kind — `knot
step --event <entry>` addresses queue entries by id, unchanged. **Pane:**
the trace-chain acceptance — one emission traced service log → commit
trailers → `tie-offs.log` → (session) with no persistent payload
duplication, green as the in-repo acceptance test and as the field-rig
run. **File set:** `src/server.rs` (startup diagnostics),
`src/adapters/outbound/event_dispatcher.rs` (deprecation), the
trace-chain acceptance test (new, in `tests/`).

## Notes

The open technical questions settled in Phase 0 (the full decisions
D1–D8 are recorded in the Phase 0 document):

- **Cutover strategy** — hard cutover of the write format, no config flag
  (D4). The service owns the write format (096's precedent is
  deprecate-and-warn for *keys*, not dual-write for *artefacts*); old
  per-knot files and dispatch directories are neither backfilled nor
  read by the new writer — they stay in git history, and startup logs a
  deprecation diagnostic naming what it finds.
- **Plan 092 interplay** — the `event:knot:<EventId>` static engine token
  and the zero-consumer diagnostics are subscription-side and survive
  unchanged; only the delivery mechanism moves off the filesystem. Phase
  2 verifies the static-token path e2e (the consumer still runs — from
  the queue entry).
- **Plan 097 coordination** — the queue-backed visibility repoints to
  the entry-carried `event_id` / `producer`, with the path-derivation
  fallback kept for legacy dispatched files during the transition (D6).
  The index stays metadata-only: the payload being *in* the queue does
  not leak into the prompt surface.
- **Truncation contract** — unchanged (D7). The tie-off content remains
  the fidelity source and the 1000-line commit-body cap stays; the slim
  event audit blocks shrink `tie-offs.log`, not the commit body.
- **Manual dispatch** — the `knot-dispatch` "drop an event file in a
  dispatch directory" manual path retires with the files (D8); the
  direct queue write is the sole manual mechanism and gains the `Event`
  kind.
- **Cross-repo repointing (not in this plan's file set)**: the
  `knot-manage`, `knot-dispatch`, `knot-inspect`, `knot-analyst` skills in
  the `agents` repo cite the per-knot paths, the dispatch directories,
  and the dead-subscription/oscillation diagnoses; loom/profile event
  URIs in rig definitions; and the `knot-template` templates (095/096 —
  the contract is hard-coded in the binary, so the template text that
  says "never write or read directly from `tie-offs/`" needs its update
  here). These land as their own planned changes in the `agents`
  project and the affected rigs.
- **Read norm**: the plan changes the artefacts; the norm ("tie-offs are
  logs — query, never whole-file read; `state.json` for snapshots; git is
  the rotation") travels with the skills repointing, not the code.
