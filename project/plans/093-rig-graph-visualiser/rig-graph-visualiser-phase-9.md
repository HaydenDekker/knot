# Phase 9 — Processing Minutes Column

## Goal

Add a **Minutes** column *beside the event count* in the strand-events
legend (phase 8): per knot, the total minutes spent in `processing`
status during the log's span, from the service log's own line-leading
timestamps. Same rows, same sort toggles, one more sortable column —
`Knot | Events | Min`.

## Data source — the status-change records (verified against the demo log)

Every record line begins with an RFC 3339 timestamp
(`[2026-09-23T11:46:41+10:00]`). A knot's busy sessions are bracketed by
the `[KNOT][STATE] change knot <loom>/<knot>: status <from>→<to>`
records:

```
[…11:46:41+10:00] [KNOT][STATE] change knot architecture-loom/architecture-gap-logger: status idle→processing strand=… event-at=…
[…11:49:31+10:00] [KNOT][STATE] change knot architecture-loom/architecture-gap-logger: status processing→completed tie-off=… event-at=…
```

Demo-log transition census (2026-10-02): busy starts
`idle→processing` ×115, `completed→processing` ×266,
`failed→processing` ×8 (= 389); busy ends `processing→completed` ×376,
`processing→failed` ×10 (= 386). The 3-session gap = knots still
`processing` at the log tail (unclosed intervals).

Counting rules:

- **Session** = one `→processing` transition until the same knot's next
  `processing→` transition. The *line-leading* timestamp is used (the
  time Knot observed the change; the inline `event-at=` is when the
  event fired and is not needed).
- Any `→processing` is a start (idle, completed, or failed); any
  `processing→X` is an end (completed or failed — a failed session still
  consumed processing time).
- **Unclosed sessions are excluded** — minutes count completed work only.
  Counting to the log tail would make the number grow while the log
  grows; re-run the extraction after the session ends for a final figure.
- Per-knot single-session invariant (Knot runs one session per knot at a
  time): a `→processing` while already busy is ignored (defensive;
  should not occur).
- **Keying:** the STATE record carries `loom/knot`; split on the last
  `/` for the bare knot id, matching the events panel's row key.
- **Rows:** the union of knots with events and knots with minutes —
  a knot that processed but has no NOTIFY events in the span (defensive;
  every start is event-driven so this should not occur) still gets a
  row with `count: 0`.

## Design

### Extraction (`scripts/rig-graph.py`)

- `parse_strand_events(log_path)` extended (same single pass over the
  file): also match
  `[KNOT][STATE] change knot <loom>/<knot>: status <from>→<to>`, parse
  the line-leading timestamp with `datetime.fromisoformat` (handles the
  `+10:00` offset).
- Per-knot state: `busy_start` (datetime or None); on start store ts, on
  end add `ts - busy_start` to the knot's busy seconds.
- Row shape changes: `{"knot", "count", "minutes"}` — `minutes` is
  closed-session seconds / 60, rounded to 1 decimal; `count` 0 for
  knots with minutes but no events. Emits unchanged: sorted by knot
  name; empty list without `--log`; missing log → exit 2.
- Console summary line gains the total:
  `wrote <path> (N nodes, M edges, K strand events, T min processing)`.

### Template (`scripts/rig-graph-template.html`)

- Third header label `Min` beside `Knot` / `Events`; the sort value
  set is `count` (default) / `name` / `minutes` — same click-to-toggle,
  active-sort-bold behaviour. `minutes` sort descends, ties by name.
- Rows: knot label left, `Events` and `Min` right-aligned (tabular
  numerals, shared class). Panel width 240px → 280px.
- Everything else (render guard, inert rows, existing interactions)
  unchanged.

## Tests

Extend `tests/fixtures/rig-graph/full/knot-service.log` with STATE
records (distinct wall times from the NOTIFY records):

| knot | sessions | minutes |
|---|---|---|
| scout | 10:01:00→10:03:30 closed (2.5) + 10:09:00→**unclosed** | 2.5 (unclosed excluded) |
| coder | 10:02:00→10:04:00 `processing→failed` (2.0) + 10:10:00→10:10:30 `completed→processing` start (0.5) | 2.5 (multi-session sum; failed end + restart start counted) |

`StrandEventsTest` additions: exact per-knot minutes (2.5 / 2.5),
unclosed-interval exclusion, multi-session sum, `processing→failed` as
an end, `completed→processing` as a start, knot with no STATE records →
`0.0`, HTML head wiring (three labels, `Min` present), no-`--log` rows
unchanged (still `[]`).

## Skill & release

- `knot-visualise` 1.5.0 → **1.6.0** (column in the contract, counting
  rules in the vocabulary section, skill description keyword).
- Version **0.55.1 → 0.57.0** path: bump **0.56.0 → 0.57.0** (new
  feature = MINOR); release notes; plan + master index updated.
- Demo regenerated with the same rig + log; headless Chromium + Firefox
  verification (column present, all three sorts correct, existing
  interactions intact).

## Implementation Status
<!-- Filled in on completion. -->
✅ Complete (2026-10-02) — v0.57.0

- [x] `parse_strand_events` single pass now also matches
  `[KNOT][STATE] change knot <loom>/<knot>: status <from>→<to>` with the
  line-leading RFC 3339 timestamp (stdlib `datetime.fromisoformat`);
  per-knot single-session state machine; any `→processing` starts, any
  `processing→X` ends (failed sessions count); **unclosed sessions
  excluded**; bare knot id from the `loom/knot` ref (last `/` segment).
- [x] Row shape `{"knot", "count", "minutes"}` — union of knots with
  events and knots with minutes; minutes rounded to 1 decimal; console
  line now `wrote <path> (N nodes, M edges, K strand events, T min
  processing)`.
- [x] Template: `Min` header label beside `Knot` / `Events`; sort set
  count (default) / name / minutes (desc, name tie-break), active-sort
  bold; minutes right-aligned with tabular numerals; panel 280px.
- [x] Tests: fixture log extended (scout 2.5 + unclosed excluded; coder
  2.5 = failed-session end + `completed→processing` restart; planner
  events-with-no-sessions → 0.0; `queue+` STATE line as non-match noise);
  3 new tests; **41/41 green**.
- [x] `knot-visualise` SKILL.md 1.5.0 → 1.6.0 (contract, vocabulary,
  description keywords); redeployed + diff-verified.
- [x] Demo regenerated: 428 strand events, **13177.9 min processing**
  (32 knots). Matches an independent per-knot ground-truth walk
  (13178.0 total, per-row rounding). Top by minutes: `lifecycle-on-delivery`
  5671.8, `plan-implementer` 2740.4, `phase-implementer` 1631.5.
- [x] Headless Chromium + Firefox: Min column present, all three sorts
  correct, toggle-back, active marker follows the sort, existing
  interactions intact, zero console errors.
- [x] Version bump 0.56.0 → 0.57.0 (Cargo.toml + release notes); plan and
  master index updated.
