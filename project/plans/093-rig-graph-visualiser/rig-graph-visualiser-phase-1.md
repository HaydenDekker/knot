# Phase 1: HTML Template + Generation

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [ ] Download minified D3 (one-time network access) and vendor it as
      `scripts/vendor/d3.min.js`
- [ ] Create `scripts/rig-graph-template.html` — the template the script reads:
      - vendored D3 inlined (no CDN)
      - `/*__GRAPH_JSON__*/` placeholder inside a `<script>` block
      - `d3-force` layout; nodes coloured by loom + legend; knot nodes as labelled
        circles, input nodes as dimmed diamonds, system node a distinct shape
      - directed edges labelled with the event name; hover tooltip (profile,
        status, last activity); pan/zoom
- [ ] Wire generation into `scripts/rig-graph.py`: read template, substitute the
      placeholder (JSON exactly once), write `--out` (create parent dirs)
- [ ] Extend `scripts/test-rig-graph.py`:
      - placeholder substitution — JSON present exactly once, no leftover marker
      - `--out` written with parent-dir creation
      - embedded JSON round-trips (extract + parse from the generated file)
      - offline guarantee — generated HTML contains no `http://`/`https://`
        references outside vendored-D3 metadata comments
- [ ] Verification — `python3 scripts/test-rig-graph.py` green (the targeted scope
      for this phase — no Rust code touched, so `cargo` gates are unaffected);
      commit sha recorded as this phase's external-verdict request. The ON-COMMIT
      Rust suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
