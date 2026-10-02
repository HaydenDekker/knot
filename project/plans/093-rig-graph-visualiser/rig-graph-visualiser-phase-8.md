# Phase 8 — Strand Events Legend (overactive-knot detector)

## Goal

A second legend panel in the generated visualisation for **identifying
overactive knots**: a sortable list (by name or by count) of knots with the
number of **strand events delivered to their strand dirs** during the log
period. The counts are extracted from the knot service log, which the user
specifies at generation time (demo:
`tie-offs/software-factory-rig/knot-service.log`).

## Data source — the strand-dir event records (verified against the demo log)

The service log records every watcher notification on a knot's strand dir as
a single-line `[KNOT][NOTIFY]` record:

```
[2026-09-23T11:46:38+10:00] [KNOT][NOTIFY] Modified /…/project/sad/README.md → Modified { loom_id: LoomId("deployment-loom"), knot_id: KnotId("sad-deployment"), strand_path: StrandPath("/…/project/sad/README.md") }
[2026-09-23T11:46:06+10:00] [KNOT][NOTIFY] Created /…/orchestrator-loom/QueueIdle/event-2026-09-24T11-46-08+10-00.md → Created { loom_id: …, knot_id: KnotId("pipeline-on-queue-idle"), strand_path: StrandPath(…) }
```

Source-selection findings (why NOTIFY and not the alternatives):

| Candidate | Verdict |
|---|---|
| `tie-offs/<rig>/events/*.json` (dispatch queue) | **Rejected** — live queue only (8 files, all current); consumed records are removed. No history. |
| Strand dirs' retained `event-*.md` files | Rejected — durable but an input directory, not a log; no timestamps of delivery, and plain-file strand dirs (e.g. `project/prds`) mix work files with events. |
| `[KNOT][EVENT]` records (knot lifecycle) | Rejected — these are the knot's *own* lifecycle events (processing/completion), not what landed in strand dirs. (This was the original misread, corrected by the user 2026-10-02.) |
| **`[KNOT][NOTIFY]` records** | **Chosen** — one record per strand-dir change, carries `knot_id` (recipient) and `strand_path` (the event file). Full history of the log's span. |

Counting rules:

- **Count kinds:** `Created` and `Modified` only. `KnotModified` (knot
  definition file changed — payload is the knot's full YAML, no
  `strand_path`) is excluded: it is not a strand event.
- **Dedup:** count **distinct `strand_path`s per knot** — a delivery that
  notifies `Created` then `Modified` (or re-notifies) is one event. Demo:
  858 Created/Modified records → 428 distinct (knot, strand_path) pairs.
- **Row identity:** the recipient **knot** (`knot_id`) — overactivity is a
  property of the consumer. Event-type (parent dir of event-file strand
  paths, e.g. `QueueIdle`) is available in the data but not a row key this
  phase.
- Whole-file scope: the log is the append-only service log (spans restarts;
  no per-run marker) — "the session" = the whole file.

## Design

### Extraction (`scripts/rig-graph.py`)

- New **optional** CLI flag: `--log <path>` — path to the knot service log.
- Parse line-by-line (stdlib only, regex per line):
  - Match `[KNOT][NOTIFY] <Created|Modified> ` records; extract
    `knot_id: KnotId("…")` and `strand_path: StrandPath("…")`.
  - Skip everything else (other record kinds, warnings, blanks,
    `KnotModified`).
- Build the set of distinct `(knot_id, strand_path)`; count per knot → new
  top-level graph-JSON field:
  `"strand_events": [{"knot": "…", "count": N}, …]` — empty list when no
  `--log` (key present either way so the template can branch on length).
- `--log` missing/unreadable → stderr message + non-zero exit (a silent
  empty legend would be misleading).

### Rendering (`scripts/rig-graph-template.html`)

- **Second panel, "Strand events"** (overactive-knot view), rendered only
  when `data.strand_events.length > 0`. Position: top-right, below
  `#title`; scrollable (`max-height: 90vh`), same visual language as the
  loom legend.
- Two columns — **Knot** and **Events** — with **clickable header labels**
  that toggle the sort:
  - **Events** (default): count descending, then knot name ascending
    (stable tie-break) — the top of the list is the overactive knot.
  - **Knot**: name ascending.
  - The active sort's header label is marked (bold / arrow glyph).
- Rows: knot name (left) + count (right, tabular numerals). Knots with no
  events are absent (the panel shows consumers, not the whole rig); knots
  seen in the log but not in the graph (renamed since) still get a row.
- Panel and rows carry distinct classes (`#events-legend`, `.event-row`)
  so the tests can pin the wiring. Does not interfere with the loom
  legend, zoom, or highlight/filter interactions.

### Tests (`scripts/test-rig-graph.py`)

- Fixture log: `Created` + `Modified` on the same path (dedup to 1),
  multiple events per knot, a `KnotModified` record (excluded),
  `[KNOT][EVENT]`/`[KNOT][STATE]` lines (ignored), a `WARNING:` line, a
  blank line → exact per-knot counts.
- `--log` missing file → non-zero exit, stderr message.
- No `--log` → `"strand_events": []`; generated HTML omits the panel.
- With `--log` → data embedded; panel wiring present (`#events-legend`,
  `event-row`, both sort toggles); offline guarantee unchanged.

### Skill (`knot-visualise`)

- Workflow step: ask for the knot service log path, pass `--log`; the panel
  is the overactive-knot detector (read top-down).
- CLI contract + interaction contract updated. Version 1.4.0 → 1.5.0;
  redeploy + diff-verify.

## Open Questions (asked 2026-10-02, round 2 — awaiting user)

1. **Row key = receiving knot** (default). The original wording said "event
   types"; given the overactive-knot goal the consumer is the unit.
   Alternative: rows = event types (event-dir names, e.g. `QueueIdle` ×N).
2. **Dedup = distinct event files per knot** (a `Created`+`Modified` pair
   is one event) — confirm, or do you want raw notification counts?
3. Demo log = `tie-offs/software-factory-rig/knot-service.log` (no
   `knot.log` exists) — confirm.

## Verification

- `python3 scripts/test-rig-graph.py` (all green).
- Regenerate the demo graph with the real log; headless Chromium + Firefox:
  panel renders, default sort = count desc, name sort toggles, counts match
  ground truth derived independently from the log, existing interactions
  intact, zero console errors.

## Implementation Status
<!-- Filled in on completion. -->
✅ Complete (2026-10-02) — v0.56.0

- [x] `--log <path>` flag on `scripts/rig-graph.py`; `parse_strand_events`
  counts distinct `(knot_id, strand_path)` pairs from `[KNOT][NOTIFY]`
  `Created`/`Modified` records (`KnotModified` and all other kinds
  excluded); `"strand_events"` graph-JSON field (empty list without
  `--log`); missing log → exit 2. Console line now reports strand-event
  total.
- [x] Template: `#events-legend` panel (top-right, below the title),
  rendered only when `data.strand_events.length > 0`; `Knot` / `Events`
  header labels toggle the sort (default: count desc, name tie-break;
  active sort bold); rows inert, tabular numerals.
- [x] Tests: 5 new (`StrandEventsTest`) — fixture log with
  Created+Modified dedup, KnotModified exclusion, other-record-kind and
  warning-line noise; per-knot exact counts; no-`--log` → empty list +
  hidden panel; missing log → exit 2; HTML panel wiring. 39/39 green.
- [x] `knot-visualise` SKILL.md 1.4.0 → 1.5.0 (CLI contract, workflow
  step 2, interaction contract row, strand-events vocabulary section,
  troubleshooting rows); redeployed + diff-verified.
- [x] Demo regenerated with the real log:
  `python3 scripts/rig-graph.py --rig software-factory-rig --log
  tie-offs/software-factory-rig/knot-service.log --out rig-graph.html`
  → 38 nodes, 85 edges, **428 strand events** across 32 knots. Top of the
  list (the overactive knots): `phase-implementer` ×35, `plan-implementer`
  ×29, `coding-test-issue-logger` ×27 — matches an independent
  `grep | sort -u | uniq -c` ground truth.
- [x] Headless Chromium + Firefox: panel visible, 32 rows, default
  count-desc order correct, name sort + toggle-back verified, head
  active-marker follows the sort, existing interactions intact, zero
  console errors.
- [x] Version bump 0.55.1 → 0.56.0 (Cargo.toml + release notes); plan
  and master index updated.
