# Phase 1: HTML Template + Generation

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] Download minified D3 (one-time network access) and vendor it as
      `scripts/vendor/d3.min.js` — done, D3 v7.9.0 (280 KB, jsDelivr)
- [x] Create `scripts/rig-graph-template.html` — done: d3-force layout,
      loom-coloured legend, knot circles / input diamonds / system node /
      unresolved dashed circles, labelled directed edges (arrow marker),
      hover tooltip (profile, status, last activity, strand-dir), pan/zoom
- [x] Wire generation into `scripts/rig-graph.py` — done: template read from
      `scripts/`, both placeholders substituted, `--out` written with parent
      dir creation
- [x] Extend `scripts/test-rig-graph.py` — done: 7 new tests (exit code,
      output written with parent-dir creation, both placeholders gone,
      `const data =` exactly once, embedded JSON round-trips, D3 inlined,
      no external network references) — 33/33 green
- [x] Verification — `python3 scripts/test-rig-graph.py` green (33/33; the
      targeted scope for this phase — no Rust code touched, so `cargo` gates
      are unaffected); commit sha `5dc6aaa` recorded as this phase's
      external-verdict request. The ON-COMMIT Rust suite is not required by
      this item.

## Deviations
- Plan said "D3 committed into the template". Instead the vendored file lives
  at `scripts/vendor/d3.min.js` and the script injects it into a `/*__D3__*/`
  placeholder at generation time (keeps the template small/readable; the
  generated output is still fully self-contained — verified by the
  no-external-references test).

## Discoveries
- The vendored d3.min.js contains W3C namespace identifier strings
  (`http://www.w3.org/...`) — namespace URIs used by DOM APIs, never fetched.
  The offline-allowlist test permits `http://www.w3.org/` and the single
  `https://d3js.org` copyright comment, and bans anything else.

## Notes
- Arrowheads are an SVG marker (`refX` offset so the tip stops short of the
  node circle); edge labels sit at the link midpoint with a white halo.
