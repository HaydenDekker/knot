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
python3 scripts/rig-graph.py --out <path.html> [--rig <dir>] [--no-state] [--json]
```

`--out` required unless `--json` (JSON to stdout, no HTML). Exit 2 with a
stderr message for a missing rig dir or no looms found.

## Test strategy

Subprocess-driven `unittest` against three fixture rigs
(`tests/fixtures/rig-graph/`): `full` exercises every `strand-dir` form
plus the state overlay and an unresolved target; `plain` covers the
no-`state.json` path and a knot with no subscribers; `dup` covers
duplicate knot ids across looms. HTML generation tests cover placeholder
substitution, parent-dir creation, embedded-JSON round-trip, and the
offline guarantee. No Rust code is touched, so the Rust suite is
unaffected (targeted Python run is the phase's verification scope).
