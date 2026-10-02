# Design: Rig Graph Visualiser (Plan 093)

The rig graph visualiser is a **standalone utility outside the Knot
binary**: a stdlib-only Python script plus a D3 HTML template plus the
`knot-visualise` sub-skill. It reads rig files and `state.json`; it never
talks to the service.

## Components

```
scripts/rig-graph.py            # extractor + generator (stdlib only)
scripts/rig-graph-template.html # D3 template (/*__D3__*/ + /*__GRAPH_JSON__*/)
scripts/vendor/d3.min.js        # vendored D3 v7.9.0 (injected at generation)
scripts/test-rig-graph.py       # unittest suite (subprocess against fixtures)
tests/fixtures/rig-graph/       # full/ (all 5 forms + state), plain/ (no state), dup/ (dup ids)
.agents/skills/knot-visualise/  # the skill (deployed to ~/.agents/skills-library/)
```

## Graph model

- **Nodes**: one per knot (`kind: knot`, coloured by loom); one per unique
  plain-filesystem `strand-dir` (`kind: input`, dimmed diamonds); one
  synthetic `knot-system` node (`kind: system`) as the source of
  rig-level `event:knot:<Event>` subscriptions; `kind: unresolved` nodes
  for subscription targets that don't exist in the rig (flagged, never
  dropped).
- **Edges** come from each knot's single `strand-dir`:
  - `event:<knot>:<Event>` → edge(s) from the producer knot
  - `event:<*-loom>:<Event>` → edges from every knot in that loom
  - `event:*:<Event>` → edges from every knot in the rig
  - `event:knot:<Event>` → edge from the system node
  - plain path → edge from the input node

  Edge labels carry the event name (or path); unresolved edges are
  flagged and rendered dashed.

## Key invariants

1. **Unique node ids.** Knot ids may repeat across looms (the repo's own
   rig does: `review-knot` in two looms). Node ids are the bare knot id
   when unique rig-wide, else `<loom>:<knot>`; labels stay bare. The
   state overlay is keyed by `(loom, knot)` from the nested
   `state.json` looms list for the same reason.
2. **Bare knot-id subscriptions match by id alone** (Knot's rule), so a
   subscription to a duplicated id fans out across looms.
3. **Offline output.** D3 is vendored and injected at generation time,
   not CDN-referenced. The generated file's only URLs are non-fetched W3C
   namespace identifiers inside D3 and its copyright comment — pinned by
   the `test_no_external_network_references` allow-list test.
4. **Service-independent.** `state.json` absence is tolerated (overlay
   simply omitted); the script never starts or queries the service.

## Frontmatter parsing

Flat `key: value` only — a hand-rolled parser splits on the first colon
(essential: `event:` URIs contain colons) and strips quotes. No PyYAML
dependency, keeping the script portable.

## CLI

```
python3 scripts/rig-graph.py --out <path.html> [--rig <dir>] [--no-state] [--log <knot-service.log>] [--json]
```

`--out` required unless `--json` (JSON to stdout, no HTML). Exit 2 with a
stderr message for a missing rig dir, no looms found, or a missing `--log`
file. The graph JSON gains a `"strand_events"` field and an
`"event_types"` field (both empty lists without `--log`, keys present
either way so the template branches on length).

## Interaction model (the generated HTML)

Filtering and highlighting are independent and compose; selection modes
are mutually exclusive (a new click replaces the previous highlight):

- **Legend checkbox** *filters* a loom — its knots and every edge touching
  a hidden knot disappear (input/system nodes stay).
- **Legend loom name** *highlights* the loom **and all nodes feeding into
  it** — one hop upstream: the source of each edge pointing at a loom
  knot (knot, input, or system sources alike). Everything else fades.
- **Node click** highlights the node, incident edges, direct neighbours,
  and the incident edges' labels. **Edge click** (wide invisible hit
  zone) highlights that edge, its endpoints, and its label. **Background
  click** clears.

## Edge rendering

- **Labels hidden by default** — dozens of 9px labels overlap and are
  illegible; they appear on hover and while their edge is highlighted.
- **Wide-scope edges render dotted** (grey): `event:<loom>:<Event>`
  (loom-wide fan-out) and `event:*:<Event>` (wildcard, label suffixed
  `· all`). A dotted arrow must not be read as "knot A sends to B" — it
  is the fan-out of B's own wide subscription. Edges carry a `scope`
  field ("loom" / "all" / absent) from the extractor.
- **Parallel edges arc** — edges sharing a node pair spread into
  perpendicular quadratic-Bézier arcs (46px/lane) so each stays
  individually clickable/hoverable. Pair counting runs **before**
  `forceLink` rewrites `source`/`target` from strings to node objects,
  and uses raw string ids.

## Activity panels (`--log`): strand events + minutes, event types

Two right-side legends (rendered only when `--log` was given and the log
has data; a flex-column container stacks them without overlapping):
the overactive-knot detector (strand events, v0.56.0, minutes column
v0.57.0) and the communication view (event types, v0.58.0).

### Strand events + minutes

- **Events column**: distinct `(knot_id, strand_path)` pairs from
  `[KNOT][NOTIFY]` `Created`/`Modified` records — a `Created` +
  `Modified` pair on one event file is one event. `KnotModified` (knot-
  definition changes, no `strand_path`) and all other record kinds are
  excluded. (The `[KNOT][EVENT]` records are the knot's own lifecycle,
  not strand-dir arrivals — the panel must not use them.)
- **Minutes column**: total busy time per knot from the line-leading RFC
  3339 timestamps. A busy session is bracketed by `[KNOT][STATE]` status
  transitions: any `→processing` (idle/completed/failed restart) starts
  one, the same knot's next `processing→` transition ends it (failed
  sessions count). **Unclosed sessions — a knot still processing at the
  log tail — are excluded**: the figure is completed work, not a number
  that grows while the log grows. **A session cannot span a service
  restart** — the append-only log carries multiple runs (each starts
  with an `initial snapshot` line), and pairing an open session from one
  run with a later run's close bills the whole shutdown to the knot
  (the v0.57.1 bug); open sessions are dropped at each run marker.
  Knots are keyed by the bare id (last `/`-segment of the `loom/knot`
  ref); rows are the union of knots with events and knots with minutes.
- **Sortable**: `Knot` / `Events` / `Min` header labels toggle the sort
  (default: Events desc, name tie-break; Min desc); the active sort is
  bold. Rows are inert.

Reading it two ways: top-down by **Events** finds the trigger magnet
(too-broad subscription); by **Minutes** finds the time sink (slow or
long-running knot — often a different knot).

### Event types (the communication view)

Per **event type**: **Inv** (distinct trigger filenames delivered — a
fan-out event delivered to several knots is *one* invocation), **Knots**
(distinct consumer knots), **Min** (the closed sessions the type caused,
attributed via each `→processing` start record's `strand=` file — same
unclosed/restart exclusions as the Minutes column).

- **Type key**: the event dir name for rig event files (paths under a
  `tie-offs/` tree, e.g. `PlanComplete` — covers both `event-*` files
  and foreign-named gate signals); the file name for project input
  files (e.g. `prd-ui-views.md`). Keyed by **bare name** — the same
  event name in several looms merges (the user's mental model is the
  event name).
- **Fan-out dedup**: when several knots subscribe to one event, the
  event file lands in each consumer's event dir under the **same
  filename** — distinct filenames, not paths, count invocations.
- **Sortable**: `Type` / `Inv` / `Knots` / `Min` header labels toggle
  the sort (default: **Min desc** — the communication-cost view; name
  tie-break); the active sort is bold. Rows are inert.

Reading it: a type with many **Inv** × many **Knots** × many **Min** is
the communication to optimise (e.g. a fan-out event that triggers many
knots, each consuming minutes).

## Gotchas (D3 v7 + the template)

- **`++` on a missing property is NaN** (spec behaviour, not an engine
  bug): `obj[key]++` when the key is absent reads `undefined`, and
  `ToNumber(undefined)` is NaN on V8 and SpiderMonkey alike — the v0.55.1
  bug: all edge paths got `QNaN,NaN` control points and browsers
  silently rejected the path data (zero lines, zero console errors).
  Use `obj[key] = (obj[key] || 0) + 1`. A regression test pins it.
- **Listeners take `(event, datum)`** — the v5-era third `index`
  argument does not exist; any selection state (e.g. "which edge is
  selected") must be tracked by **object identity** (`activeEdge === d`),
  not by index.
- **Legend rows use the join shape** `selectAll("div.loom-row").data(...)
  .enter().append("div")` — a single `append("div").data(looms)` binds
  only the first datum, so the legend rendered one row (the v0.52.1
  bug). Same rule applies to the activity panels' rows.
- **Event-type attribution rides on `strand=`** — every
  `→processing` start record (389/389 in the demo log, including
  `failed→processing` retries) carries `strand=<triggering file>`;
  the session's duration is billed to that file's event type. A start
  without it (defensive) still counts in the knot's minutes but is
  attributed to no type.

## Test strategy

Subprocess-driven `unittest` against three fixture rigs
(`tests/fixtures/rig-graph/`): `full` exercises every `strand-dir` form
plus the state overlay and an unresolved target, and carries a fixture
service log (`knot-service.log`) with strand-event records (Created +
Modified dedup, KnotModified exclusion, warning/blank noise, a
fan-out event delivered to two knots under the same filename, a
project input file), STATE records (every start carries `strand=`;
closed sessions, multi-session sums, `processing→failed`,
`completed→processing` restart, an unclosed session, a `queue+` noise
line, an open session dropped at an `initial snapshot` run marker), and
a knot with events but no sessions; `plain` covers the
no-`state.json` path and a knot with no subscribers; `dup` covers
duplicate knot ids across looms. HTML generation tests cover placeholder
substitution, parent-dir creation, embedded-JSON round-trip, the offline
guarantee, the interaction wiring (filter/highlight/sort markers in the
emitted JS), the `++`→NaN regression guard, and the activity-panels
wiring (strand events + event types: both panels in the container, the
guards, the sort toggles, the exact fixture-derived row data — 44 tests). No Rust code is touched, so the Rust suite is unaffected
(targeted Python run is the phase's verification scope). Runtime
behaviour of the generated HTML is verified in headless Chromium and
Firefox (Playwright) when a rig project with the browsers available is at
hand.
