# Plan: Rig Graph Visualiser

## Implementation Status: ✅ Complete (2026-10-02) — phases 0–3 in v0.51.0; phase 4 in v0.52.0 (bugfix v0.52.1); phase 5 in v0.53.0; phase 6 in v0.54.0; phase 7 (arced parallel edges + dotted-only loom marker) in v0.55.0 (bugfix v0.55.1)

## Notes
- All 4 phases (0–3) implemented and verified (33/33 Python tests green)
- Version bumped to 0.51.0
- Design knowledge in `project/design/design-rig-graph-visualiser.md`
- `knot-visualise` skill deployed to `~/.agents/skills-library/`; updated
  `knot` master deployed to `~/.agents/skills/knot/` (diff-verified)
- No rig-document migration (knot-update carries no 0.51.0 entry)
- Phase 4 (added 2026-10-02 at user request): interactive legend (loom
  filter checkboxes + loom highlight on click), node/edge click
  highlighting, background-click clear; skill 1.1.0 interaction contract;
  released in v0.52.0
- Phase 5 (added 2026-10-02 at user request): loom legend highlight now
  shows the target loom **and all nodes feeding into it** (source of each
  edge pointing at a loom knot, one hop upstream); skill 1.2.0; released
  in v0.53.0
- Phase 6 (added 2026-10-02 at user request): edge labels hidden by
  default, revealed on hover/highlight; loom/wildcard wide-scope
  subscriptions marked dotted with `· loom` / `· all` so fan-out arrows
  cannot be misread as knot-specific sends; skill 1.3.0; released in
  v0.54.0
- Phase 7 (added 2026-10-02 at user request): parallel edges between a
  node pair spread into perpendicular arcs (bidirectional pairs no longer
  overlap as one double-headed edge); the `· loom` label suffix dropped
  in favour of the dotted style alone (`· all` kept for wildcards);
  skill 1.4.0; released in v0.55.0

### Bugfix: Phase 4 legend rows and edge-click highlighting (2026-10-02)

User verification of the v0.52.0 output found two Phase 4 defects: only one
legend row rendered, and clicking an edge produced no highlight.

- **What was wrong:** (1) the legend built its rows with
  `legend.append("div").data(looms)` — `append` creates a *single* element,
  so only the first loom's datum bound (one row). (2) the edge hit-line
  listener used a d3 v5-era `(event, datum, index)` signature; D3 v7
  listeners receive only `(event, datum)`, so the edge index was
  `undefined` and the highlight state never engaged.
- **How it was fixed:** rows built via `selectAll("div.loom-row").data(looms)
  .enter().append("div")` (one row per loom), and the edge selection is now
  tracked by object identity (`activeEdge`, matched with `===`) instead of
  an index. Wiring tests updated: `activeEdge` present, `activeEdgeIndex`
  absent, `.data(looms).enter()` marker asserted. Released in v0.52.1.

### Bugfix: Phase 7 arc counter stored NaN — no edges rendered (2026-10-02)

User verification of the v0.55.0 output found the graph rendered no lines
and no edge labels at all (nodes and the force layout were fine).

- **What was wrong:** the pair counter used `pairCount[e._pair]++` on a
  missing property. Contrary to the "undefined counts as 0" intuition,
  `++` on `undefined` goes through `ToNumber` — `undefined → NaN` — so
  every counter value stored `NaN` (confirmed identical on both V8 and
  SpiderMonkey). `arcGeom` then took the arc branch for every edge
  (`n === 1` is false for NaN) with a NaN control point, producing path
  data like `M…QNaN,NaN …`; the browser rejects such path data, so no
  edge path or label position rendered. The Python test suite could not
  catch this — it verifies template markers, not JS semantics.
- **How it was fixed:** the count is now built with an explicit
  undefined guard — `pairCount[e._pair] = e._idx + 1` (where
  `e._idx = pairCount[e._pair] || 0`) — which is engine-independent.
  Regression test added: the guarded increment asserted present and the
  raw `pairCount[e._pair]++` pattern asserted absent. Verified in
  headless Chromium and Firefox: 85/85 edge paths valid, 0 NaN paths,
  the 3 multi-edge pairs arc, single-edge pairs stay straight, legend
  highlight and hover interactions intact, no console errors.
  Released in v0.55.1.

## Problem

A rig's structure — which knots exist, which looms they belong to, and how
they are wired together via `event:` subscriptions — is currently only
visible by reading many small `.md` files across `rig/*-loom/` and tracing
`strand-dir` declarations by hand. There is no way to see the whole rig's
producer→consumer topology at a glance.

## Target

A single Python script and an accompanying skill that:

1. **Extracts the rig graph** — knots (nodes) and their event
   subscriptions (edges) — from the rig's `.md` files.
2. **Renders it to HTML** — a self-contained, offline-viewable D3
   visualisation, written to a user-specified location.
3. **Is packaged as a skill** — `knot-visualise` — so an agent can invoke
   the visualisation workflow in any project with a rig.

No Knot binary changes. No network access required at view time.

## Design

### Graph model

- **Nodes**
  - *Knot nodes* — one per knot `.md` file in a loom directory
    (directories ending `-loom` under the rig root). Attributes: `id`,
    `loom`, `agent-profile-ref`, `strand-dir`, and (when available)
    runtime `status` + `last_event_at` from `tie-offs/<rig>/state.json`.
  - *Input nodes* — one per unique plain-filesystem `strand-dir`. Rendered
    dimmer; they are the rig's external entry points.
  - *System node* — a synthetic `Knot (system)` node, source of edges for
    rig-level `event:knot:<Event>` subscriptions.
- **Edges** — derived from each knot's `strand-dir` by subscription type:
  | `strand-dir` form | Edge(s) |
  |---|---|
  | `event:<knot>:<Event>` | producer knot → consumer |
  | `event:<*-loom>:<Event>` | every knot in that loom → consumer |
  | `event:*:<Event>` | every knot in the rig → consumer |
  | `event:knot:<Event>` | system node → consumer |
  | plain path | input node → consumer |

  Edges carry the event name (or path) as a label. Subscriptions whose
  target knot/loom does not exist in the rig are recorded with an
  `unresolved: true` flag rather than dropped.

### Script: `scripts/rig-graph.py`

- Python 3, **stdlib only**. Frontmatter is flat `key: value` YAML; a
  small hand-rolled parser (split on `---`, parse `key: value` pairs,
  strip quotes) is sufficient — no PyYAML dependency.
- CLI:
  ```
  python3 scripts/rig-graph.py --out <path.html> [--rig <dir>] [--no-state]
  ```
  - `--out` — **required** output path for the HTML file (created
    in-place; parent dirs created if missing).
  - `--rig` — rig directory, default `rig/` (relative to CWD).
  - `--no-state` — omit the runtime state overlay. Default behaviour:
    overlay `status`/`last_event_at` if `tie-offs/<rig>/state.json`
    exists; the script must tolerate its absence (service not running).
- Pipeline: parse looms/knots → resolve edges → build graph JSON
  (`{"nodes": [...], "edges": [...]}`) → substitute the JSON into the
  template's placeholder → write the output file.
- Exit non-zero with a clear message when the rig dir is missing or no
  looms are found.

### Template: `scripts/rig-graph-template.html`

- A single HTML file holding the D3 visualisation with a
  `/*__GRAPH_JSON__*/` placeholder in a `<script>` block.
- **D3 is vendored inline** (a minified `d3` copy downloaded once and
  committed into the template) so the generated file is fully
  self-contained and viewable offline — no CDN at view time.
- Rendering: `d3-force` layout; nodes coloured by loom with a legend;
  knot nodes as labelled circles, input nodes as dimmed diamonds, the
  system node as a distinct shape; edges as directed arrows labelled
  with the event name; hover tooltip showing knot metadata
  (profile, status, last activity); pan/zoom.

### Skill: `knot-visualise`

- New sub-skill at `.agents/skills/knot-visualise/SKILL.md`. Teaches the
  agent: where the script lives, the CLI contract, graph-model
  vocabulary (node/edge kinds, unresolved edges), how to choose the
  output location with the user, and how to interpret the result
  (loops, fan-out, orphaned inputs, unresolved targets).
- `knot` master router: add a `knot-visualise` row to the routing table
  and to its DO-NOT-USE cross-references.
- AGENTS.md: add `knot-visualise` to the Knot Skills list and to the
  deploy/verify copy loops.

### Skill deployment policy (per project convention)

The skill is developed and tested locally at project level
(`.agents/skills/`). On plan completion and commit, it is copied to the
production locations (`~/.agents/skills-library/knot-visualise/` as a
sub-skill, plus the updated `knot` master to `~/.agents/skills/knot/`)
with diff verification, exactly as the existing skills are deployed
(see AGENTS.md "Skill Installation").

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| Rust lib + integration suite | Knot binary behaviour (config parsing, event dispatch, state writes) | ✅ Green — unaffected; no binary changes in this plan |
| (none) | No existing tests cover rig-graph extraction or HTML generation — this is a new, binary-independent utility | N/A |

## Test Gaps

- No test for rig graph extraction (node set, edge resolution per
  `strand-dir` form, unresolved targets, state overlay).
- No test for HTML generation (placeholder substitution, embedded JSON
  round-trip, offline/no-network-refs guarantee).
- No fixture rig covering all five `strand-dir` subscription forms.

## Phases

### Phase 0: Graph extraction script

- Create `scripts/rig-graph.py` implementing the frontmatter parser,
  rig walk, edge resolution (all five `strand-dir` forms), state overlay,
  and graph JSON construction, plus the CLI.
- Support `--json` (print graph JSON to stdout, skip HTML) to make the
  extraction independently testable before the template exists.
- **Test fixtures**: a mock rig tree under `tests/fixtures/` covering:
  knot-level / loom-level / wildcard / rig-level / plain-path
  subscriptions, an unresolved target, a knot with no subscribers, and a
  state.json overlay case (plus the no-state.json case).
- **Tests**: fixture-driven checks on the emitted graph JSON (node set,
  edge set with labels, unresolved flags, state overlay applied/omitted,
  missing-rig error exit code).

### Phase 1: HTML template + generation

- Vendor minified D3; author `scripts/rig-graph-template.html` with the
  force layout, styling, legend, tooltips, and pan/zoom around the
  `/*__GRAPH_JSON__*/` placeholder.
- Script writes the substituted file to `--out`.
- **Tests**: placeholder substitution (JSON present exactly once, no
  leftover marker); output file written with parent dir creation; the
  embedded JSON round-trips (extract and parse from the generated file);
  output contains no network references (offline guarantee — scan for
  `http://`/`https://` outside the vendored D3's own metadata comments).

### Phase 2: Skill authoring

- Write `.agents/skills/knot-visualise/SKILL.md` (workflow, CLI
  contract, interpretation guide).
- Update `.agents/skills/knot/SKILL.md` routing table.
- Update `AGENTS.md` (skills list + deploy/verify loops).
- **Verify**: skill frontmatter matches the house format of sibling
  skills; router and AGENTS.md lists are consistent.

### Phase 3: End-to-end check and deployment

- Run the script against the real rig in this repo, writing to a
  scratch path (e.g. `/tmp/rig-graph.html`); sanity-check the rendered
  graph JSON and that the HTML opens standalone.
- Commit the work.
- Deploy `knot-visualise` to `~/.agents/skills-library/` and the updated
  `knot` master to `~/.agents/skills/knot/`, with diff verification per
  AGENTS.md.
- Mark the plan complete (via `project-plan-completion`).

### Phase 4: Interactive legend and click highlighting (added 2026-10-02)

Extend the D3 template so the graph is explorable by selection:

- **Legend loom rows carry a checkbox** — unchecking a loom **filters** it
  out: its knot nodes and every edge touching a hidden knot disappear
  (input and system nodes stay visible; the simulation is untouched).
- **Clicking a loom's name in the legend highlights** that loom: its knot
  nodes and every edge incident to them stay at full opacity, everything
  else fades. Click again (or click the background) to clear.
- **Clicking an edge highlights** that edge plus its two connected nodes
  (a wide invisible hit line makes thin edges clickable); clear by
  clicking the background.
- **Clicking a node highlights** the node, its incident edges, and its
  direct neighbours; clear by clicking the background.
- Selection modes are mutually exclusive — a new click replaces the
  previous highlight; filtering (checkboxes) and highlighting (clicks)
  are independent and compose.
- Tests: generated-HTML wiring assertions (checkbox generation, hit
  lines, highlight/filter functions, background clear) plus the existing
  suite; skill docs updated with the interaction contract and re-deployed.

### Phase 5: Loom highlight includes all feeding nodes (added 2026-10-02)

When a loom is highlighted (legend name click), only the loom's own knots
stayed at full opacity; the nodes **feeding into** the loom (the sources of
edges pointing at its knots) faded, so the loom's inputs were visible as
arrows but not as named nodes.

- **Loom highlighting now shows the target loom and all nodes feeding
  into it**: the loom's own knots, every incident edge, and the **source
  node of every edge whose target is a knot of the loom** (one hop
  upstream, following edge direction backwards) — knot, input, or system
  sources alike. No downstream targets, no further expansion.
- Node-click and edge-click highlighting are unchanged (both already
  include their neighbour endpoints).
- Skill docs updated (interaction contract notes the feeding nodes);
  wiring test marker added; generated artefacts regenerated.

### Phase 6: Edge event labels on hover/highlight + wide-scope marker (added 2026-10-02)

User asked which event `lifecycle-on-block` sends to `plan-implementer`
and could not tell. Diagnosis: every edge label is drawn at all times —
85 tiny 9 px grey labels overlapping — so no label can be attributed to
its edge, and there is no edge-hover or edge-highlight label behaviour
at all. Additionally, `event:<loom>:<Event>` subscriptions fan out to
every knot of the loom and render as knot-specific arrows (e.g.
`plan-implementer` subscribes `event:planning-loom:PlanReady`, which
draws one arrow from *every* planning-loom knot), reading as if each knot
individually sends the event.

- **Labels hidden by default; revealed on hover and while highlighted**:
  hovering an edge (wide 10 px hit line, identity-based label lookup —
  D3 v7 listeners take no index) lights that label; any active highlight
  (edge, node, or loom) lights the labels of the edges in the set via a
  `lit` class.
- **Wide-scope subscriptions are marked**: loom-level (`event:<loom>:…`)
  and wildcard (`event:*:…`) fan-out edges carry `scope` (`"loom"` /
  `"all"`), render dotted, and their labels are suffixed `· loom` /
  `· all` — so "knot A sends to knot B" is never confused with
  "knot B listens to the whole loom".
- Skill docs updated (reading guide: hover/highlight reveal the event
  name; dotted = wide subscription); wiring + extractor test markers
  added; generated artefacts regenerated.

### Phase 7: Arced parallel edges (added 2026-10-02)

User saw an "edge with arrowheads at both ends" between
`lifecycle-on-delivery` and `plan-implementer`, whose arrowhead at one
end vanished when clicked. Diagnosis: two **parallel edges in opposite
directions** (the `PlanReady` and `ImplementationNote` loom fan-outs)
drew as straight lines between the same two points — perfectly
overlapping, so the pair read as one double-headed edge, clicks hit
whichever was topmost, and the loser's arrowhead faded out of view.

- **Edges between a node pair with more than one edge are spread into
  perpendicular quadratic arcs** (each edge its own lane, ~46 px apart
  per lane); lone edges stay straight. Edges are now `path` elements
  (lines can't arc); hit zones and labels follow the same arcs, labels
  at the arc midpoint.
- **Clicks, hover, highlighting, and filtering all operate on the
  separated edges.**
- **The redundant `· loom` label suffix is dropped** (user request):
  the dotted style alone marks a loom-wide subscription; only the
  wildcard scope keeps a label suffix (`· all`).
- Skill docs updated (parallel edges arc; dotted = loom-wide, `· all` =
  wildcard); wiring test markers added; generated artefacts regenerated.

## Notes

- **Out of scope**: Knot binary changes (the rig files and state.json are
  the only inputs; nothing runs the service); server/live-update mode
  (the visualisation is a snapshot); editing looms/knots from the HTML.
- Vendoring D3 requires one-time network access to download
  `d3.min.js` during development; the generated output itself is fully
  offline.
- The script must never require the Knot service to be running.
