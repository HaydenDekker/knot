# Phase 10 — Event Types Panel (communication view)

## Goal

A third view beside the strand-events panel: **event types** — the
edges of the communication graph, not the knots. For each event type
(e.g. `PlanComplete`): how many times it fired (**invocations**), how
many knots it touched (**consumers**), and how much knot processing it
**caused** (**minutes**). The question it answers: *can the
communications be optimised?* (a type with many invocations × many
consumers × many minutes is the communication bottleneck).

## Data source (verified against the demo log, 2026-10-02)

The key enabler: **every `→processing` start record carries
`strand=<triggering event file>`** — 389/389 starts in the demo log,
including `failed→processing` retries. So a closed session is
attributable to the exact event that caused it.

- **Event type** from the trigger/strand path:
  - path contains `/tie-offs/` (a rig event dir, e.g.
    `…/tie-offs/<rig>/<loom>/PlanComplete/event-….md`) → type = the
    **parent dir name** (`PlanComplete`). This covers both `event-*`
    files and foreign-named gate-signal files (`GateFailureSignal`,
    `ExternalGateCompleted` — timestamped files under a rig event dir).
  - otherwise (project input dirs, e.g. `project/prds/prd-ui-views.md`)
    → type = the **file name** (a work file is its own trigger).
  - Type is keyed by **bare name** — `PlanComplete` exists in three
    looms (validation/test-planning/deployment) and the user's mental
    model is the event name; the counts merge across producer looms.
- **Invocations** = distinct trigger **file names** delivered for the
  type (from the `[KNOT][NOTIFY]` records). A fan-out event is delivered
  as copies under each consumer's event dir with the **same filename**
  (verified: e.g. `event-2026-09-23T11-54-50+10-00.md` in several
  dirs) — so distinct filenames = logical events, counted once, not per
  consumer.
- **Consumers** = distinct knots that received a delivery of the type
  (NOTIFY `knot_id`).
- **Minutes caused** = sum of the durations of **closed** sessions
  whose start record's `strand=` file belongs to the type — same
  session rules as the Minutes column (failed sessions count, unclosed
  sessions excluded, open sessions dropped at each `initial snapshot`).
  A start without `strand=` (defensive; 0/389 in the demo) is still
  counted in the knot's minutes but attributed to no type.

## Design

### Extraction (`scripts/rig-graph.py`)

- `parse_strand_events` (same single pass) now also:
  - per type: `set` of delivered filenames, `set` of consuming knots
    (from NOTIFY), and busy seconds (from closed sessions, via the
    start record's `strand=` — captured as `(ts, type)` in
    `busy_start`).
- New graph-JSON field, sibling of `strand_events`:
  `"event_types": [{"type", "invocations", "consumers", "minutes"}, …]`
  — empty list without `--log` (key present either way).
- `parse_strand_events` returns both row lists; `main()` assigns both
  fields.

### Template (`scripts/rig-graph-template.html`)

- The right-side panels move into a flex-column container
  (`#side-panels`) so they stack without overlapping:
  **Strand events** (existing, top) and **Event types** (new, below).
  The container renders only when at least one panel has data.
- Event-types panel: columns `Type | Inv | Knots | Min`, sortable via
  the header labels (same click-toggle, active-bold behaviour).
  **Default sort: Min descending** — this is the communication-cost
  view; `Inv` / `Knots` / `Type` for the other perspectives. Rows inert.
- Existing strand-events panel: unchanged behaviour, un-positioned into
  the container.

## Tests

Fixture log (`full/knot-service.log`) reworked so every start carries
`strand=` and exercises the type rules:

- fan-out: `ReviewDone/event-7.md` delivered to coder **and** planner
  (one invocation, two consumers, both sessions attributed).
- input file: scout session triggered by `project/prds/prd.md`
  (type `prd.md`).
- events-but-no-sessions: `reviewer` receives `ReviewDone/event-4.md`
  (type minutes 0 for its share; knot minutes 0.0).
- unclosed (scout's `PlanReady/event-2.md`) and
  restart-dropped (watchdog's `ReviewDone/event-2.md`) sessions
  contribute **no** type minutes.

Expected `event_types`: `PlanReady {3, 1, 2.5}`,
`ReviewDone {5, 3, 5.0}`, `prd.md {1, 1, 1.0}` (name-sorted).
Existing strand-events expectations updated (coder 4 events / 4.0 min,
planner 3 / 1.0, scout 3 / 3.5, reviewer 1 / 0.0). New tests: exact
`event_types` list, restart/unclosed exclusion at type level,
no-`--log` → `[]`, HTML wiring (container, headers, `Min` default
sort, guard).

## Skill & release

- `knot-visualise` 1.6.0 → **1.7.0** (contract row, vocabulary,
  description keywords).
- Version **0.57.1 → 0.58.0** (new feature = MINOR); release notes;
  plan + master index updated.
- Demo regenerated with the same rig + log; headless Chromium + Firefox
  verification (panel renders, all sorts, stacking, existing
  interactions intact).

## Implementation Status
<!-- Filled in on completion. -->

**Status: COMPLETE** — v0.58.0, 2026-10-02.

- Extractor: `_event_type_of` (tie-offs dir → parent dir name; input
  file → file name, bare-name keyed); `parse_strand_events` now
  returns both row lists — knot rows and type rows (invocations =
  distinct trigger filenames from NOTIFY, consumers = distinct knots,
  minutes via the start record's `strand=` attribution; same
  unclosed/restart session rules). `--json` emits `event_types`.
- Template: `#side-panels` flex-column container (stacks without
  overlapping); the event-types panel (`Type | Inv | Knots | Min`)
  renders below the strand-events panel, sortable, **default Min
  descending**; container shows only when a panel has data.
- Fixture log reworked: every start carries `strand=`; fan-out
  (`ReviewDone/event-7.md` → coder + planner, one invocation, two
  consumers), input file (`brief.md`), events-without-sessions
  (`reviewer`), unclosed (scout 10:25:00) and restart-dropped
  (watchdog) sessions. Expected type rows: PlanCreated {1,1,0.0},
  PlanReady {3,2,2.5}, ReviewDone {4,2,5.0}, brief.md {1,1,1.0}.
- Tests: 44/44 green (new: `test_event_types_counts`,
  `test_event_types_exclude_unclosed_and_restart_sessions`; updated:
  counts/minutes expectations, zero-minutes knot → reviewer, no-`--log`
  → both lists empty, HTML wiring asserts both panels).
- Skill 1.7.0; release notes v0.58.0; demo regenerated (24 event types;
  PlanReady 693.3 min tops the cost view, PlanComplete 18×3 the top
  fan-out); headless Chromium + Firefox verified (all sorts, stacking,
  35 nodes/171 paths, zero page errors).
- Commit on main: `feat(rig-graph): event-types panel — invocations, consumers, minutes per event type (v0.58.0)`.
