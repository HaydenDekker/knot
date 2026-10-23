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

Outline level — re-verified in Phase 0. Known relevant surfaces:

| Test | What it covers | Status |
|------|---------------|--------|
| `tests/consolidated_log.rs` | Binary-level: single-line `[KNOT][EVENT]`/`[KNOT][STATE]` stderr grammar; `state.json` change-driven rewrites (plan 083) | ✅ Green — will be extended for the new trace fields |
| `tests/event_fanout.rs` | Event fan-out to subscribers | ✅ Green — pins current (dispatch-file) behaviour; the migration target |
| `tests/event_enforcement.rs` | Per-event enforcement / follow-up tie-off append (plans 091/094) | ✅ Green — the follow-up append must re-target `tie-offs.log` |
| `tests/agent_integration.rs` | End-to-end knot processing through pi | ✅ Green — the e2e pane for the queue + log changes |

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

Outline — phase panes and file sets to be firmed up in Phase 0.

### Phase 0: Technical exploration and baseline

Pin the code surface (tie-off append path, event parser, queue adapter,
git commit path), re-verify the test table, and decide the open technical
questions (see Notes). Produces the phase file sets; no behaviour change.

### Phase 1: Payload-embedded queue entries

The new queue-entry shape carries the event payload in the entry body,
fan-out one entry per subscriber, removal at completion/failure,
`load_persisted` redelivery, and the direct-queue-write path for manual
dispatch of the new kind. Pane: the fan-out/queue test pane green.

### Phase 2: Direct queueing from the tie-off parse

Events parsed from a producer's tie-off queue directly; the dispatched
`.md` files and the `event:`-URI strand/watch path for events are no
longer created. Pane: e2e — producer tie-off → consumer processed, with
the dispatch-dir-absent assertions green.

### Phase 3: Commit trailers and structured trace fields

Knot commits carry the trailer fields (`session`, `event`, `source`,
`original_strand`); the service-log line grammar gains the same fields.
Pane: commit-trailer + consolidated-log grammar tests green.

### Phase 4: The unified `tie-offs.log`

The service appends every tie-off section (trigger header reworked into
the structured first line, content, slim event audit blocks — id,
occurred, reason, timestamp; no payload bodies) to one
`tie-offs/<rig>/tie-offs.log`, serialised, sole writer. Per-knot files
retire; the enforcement follow-up append re-targets the unified log.
Pane: consolidated-log binary-level test pane green.

### Phase 5: Cutover and migration

One planned change, never piecemeal: existing rig layouts cut over (old
per-knot files stay in git history, no backfill — the service owns the
write format), repoint the references (see Notes for the cross-repo
surface), deprecate the retired paths with diagnostics. Pane: a
field-rig acceptance run — the full trace chain verified end to end.

## Notes

Open technical questions for Phase 0 exploration:

- **Cutover strategy**: hard cutover vs. a config flag (the rig has a
  history of deprecate-with-warning key handling — 096 retired keys by
  parse-warn-not-honour). Decide before Phase 2.
- **Plan 092 interplay**: the `event:knot:<EventId>` static engine token
  and zero-consumer diagnostics reference the subscription mechanism this
  plan changes — check what survives and what repoints.
- **Plan 097 (in progress)**: queue-backed pending-event visibility reads
  the queue; payload-embedded entries change what the visibility surface
  shows (the payload is now *in* the queue). Coordinate ordering.
- **Truncation contract**: the 1000-line commit-body truncation. With
  slim event blocks the sections shrink; decide whether the file remains
  the fidelity source (current stance) or the limit moves.
- **Manual dispatch**: the `knot-dispatch` "drop an event file in a
  dispatch directory" manual path retires; the direct queue write becomes
  the sole manual mechanism and gains the new event kind.
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
