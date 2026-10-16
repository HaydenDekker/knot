# Plan: Event-enforcement follow-up tie-off is never appended

**Status:** ✅ Complete (2026-10-16) — released in v0.59.0
**Bug report:** when a session produces a tie-off that fails event
enforcement (required event blocks missing — the "malformed" tie-off case),
Knot re-injects the tie-off request and asks the agent to acknowledge the
missing events. The follow-up's events *are* dispatched to consumers (the
service log shows them), but the follow-up response is **never appended to
the tie-off file** — reading `tie-offs/<rig>/<loom>/tie-off-<knot>.md` shows
only the first (malformed) tie-off. The second tie-off does not re-appear.

## Root cause

Trace of the success path for one strand run:

1. `ProcessStrand::execute` (`src/application/usecases/process_strand.rs`)
   runs the agent (`session_resume::execute_with_resume`), then writes the
   **only** successful-run tie-off section via
   `process_strand_helpers::write_tie_off`
   (`process_strand.rs:498` → `tie_off_sink.append`), and *then* calls
   `handle_success`.
2. `handle_success`
   (`src/application/usecases/process_strand_helpers.rs`, "Event
   Enforcement" block, ~line 628) detects the missing event blocks, logs
   `KnotEventsMissing`, and re-enters the session once with
   `session_resume::inject_event_request` (the re-injected tie-off request,
   naming the missing blocks and repeating the listener context).
3. On `Ok(response)`, `handle_success`:
   - parses the follow-up for event blocks
     (`tieoff_parser::extract_agent_events`),
   - dispatches them to consumer knots,
   - recomputes the still-missing set (and logs a second
     `KnotEventsMissing` if any remain) —
   - but **never calls `tie_off_sink.append`**. The raw `response` is
     consumed in place and dropped.

`inject_event_request` (`src/application/session_resume.rs`) returns the
follow-up's raw stdout; persistence is the caller's job, and the caller
forgot it. The production tie-off appends are exactly two call sites —
the error tie-off (`process_strand.rs:436`) and `write_tie_off`
(`process_strand.rs:498`) — and neither covers the enforcement follow-up.

This is why the tie-off log shows only the first section: `KnotEventsMissing`
lines and the dispatched follow-up events make the re-injection visible in
the *service* log, but the *tie-off* file keeps the first malformed section
only, so the follow-up's acknowledgement blocks are untraceable in the
document that is supposed to "tell the complete story of what has happened
to a strand" (see `tie-off-append-and-event-context.md`, PRD story 6).

Nothing else is affected: the follow-up's events are dispatched correctly,
and enforcement runs before the late event-file removal and the git commit,
so once the second section is written it will also be captured by the
git-versioned commit (no reordering needed).

## Target

- The event-enforcement follow-up's response is appended to the **same**
  tie-off file as a second `## …` section (after the `---` delimiter),
  exactly like a normal append via `FileSystemTieOffSink::append`.
- The section is identifiable as the enforcement follow-up (distinct
  event label in the header), carries the same session ID as the first
  section, and is written for *every* successful re-entry — including an
  empty follow-up response (as a short marker section), so the tie-off
  file and the `KnotEventsMissing` records always agree.
- The `Err` branch (no session ID / runner error) stays as-is: best-effort,
  `eprintln!`, no section (there is no agent response to record).

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Where to append | `handle_success`, top of the `Ok(response)` branch — before dispatch | Mirrors the main path order (tie-off written before `dispatch_agent_events`); the tie-off is the record of the turn, the dispatch derives from it |
| Path | The same `tie_off_path` (already a `handle_success` parameter) | One knot = one tie-off file; the append machinery (`---` delimiter) already handles the existing file |
| Header label | `event_type = Some("{event_label} (event-enforcement follow-up)")` | The section header is `## {knot_name} triggered by {event_type} {strand}` — reusing the plain label would make the follow-up look like a duplicate trigger; the label is the core observability fix |
| Status | `TieOffStatus::Produced` | The follow-up is a successful agent turn |
| Session ID | `resolved.session_id` (same pi session — the follow-up re-enters it via `--session-id`) | Same session, so the same ID is truthful |
| Empty follow-up | Append a marker section: `"(event-enforcement follow-up produced no response)"` | First-turn convention: an empty response still gets a recorded tie-off (the "no final response" failed tie-off). The second `KnotEventsMissing` (still-missing = all) is then always paired with a visible section |
| `Err` branch | No append | No agent output exists; the `eprintln!` + the first `KnotEventsMissing` remain the record |
| `event_metadata` / `strand_path` | Re-extract from the same `strand_path` (`extract_event_metadata`), same as `write_tie_off` | Same strand trigger |

## Changes

### Phase 0: Failing tests (test fixture + new tests)

**File:** `src/application/usecases/process_strand.rs` (test module)

`build_enforcement_strand` (`process_strand.rs:6749`) already constructs a
`TrackingTieOffSink` but discards its appends handle (`_`). Change the
return tuple to include `tie_off_appends: Arc<Mutex<Vec<TieOff>>>` and
update the ~6 existing enforcement tests to the new arity (no assertion
changes).

**New tests:**

| Test name | Assertion |
|-----------|-----------|
| `process_strand_enforcement_followup_writes_second_tieoff_section` | First output acknowledges `PlanCreated` only; follow-up emits the `PlanRejected` block → **two** appends to the same `TieOffPath`; second content contains the `PlanRejected` block; second `event_type` contains `event-enforcement follow-up`; second `session_id == Some("sess-test")`; second `status == Produced` |
| `process_strand_enforcement_followup_still_partial_appends_section` | Follow-up emits only an unrelated block → second section is still appended (story complete) *and* the second `KnotEventsMissing` is logged (existing `process_strand_enforcement_followup_still_partial_logs_second` gains the append assertion instead of a new test, if the fixture change makes that simpler) |
| `process_strand_enforcement_followup_empty_response_appends_marker` | Follow-up returns `""` → second append with the marker content; second `KnotEventsMissing` still missing = all expected |
| `process_strand_enforcement_followup_error_no_second_tieoff` | First output carries **no** session ID → `inject_event_request` errors → exactly **one** append (first section only); existing behaviour preserved |

- [ ] Extend `build_enforcement_strand` to expose `tie_off_appends`
- [ ] Update existing enforcement tests for the new arity
- [ ] Add the four tests above (they fail before Phase 1)
- [ ] `cargo test` — new tests fail for the right reason (second append absent)

### Phase 1: Implementation

**File:** `src/application/usecases/process_strand_helpers.rs`
(`handle_success`, `Ok(response)` branch)

At the top of the `Ok(response) =>` arm, before the dispatch block:

```rust
// Persist the follow-up turn: append it as a second section to the
// same tie-off file (best-effort — observability, like write_tie_off).
let followup_content = if response.trim().is_empty() {
    "(event-enforcement follow-up produced no response)".to_string()
} else {
    response.clone()
};
let followup_tie_off = TieOff {
    content: followup_content,
    path: tie_off_path.clone(),
    status: crate::domain::entities::TieOffStatus::Produced,
    knot_name: Some(knot.id.0.clone()),
    event_type: Some(format!("{event_label} (event-enforcement follow-up)")),
    strand_path: Some(strand_path.0.display().to_string()),
    timestamp: None,
    agent_events: Vec::new(),
    event_metadata: extract_event_metadata(strand_path).unwrap_or_default(),
    session_id: resolved.session_id.clone(),
};
let _ = ps.tie_off_sink.append(followup_tie_off);
```

(Exact shape to be finalised against the current `TieOff` fields at
implementation time; `handle_success` already has `tie_off_path`,
`strand_path`, `event_label`, `knot`, `resolved` in scope.)

- [ ] Implement the append as above
- [ ] `cargo test` — all new tests green, enforcement suite green

### Phase 2: Documentation

- [ ] `docs/concepts.md` — "Agent Events and Enforcement": add a sentence
      that the follow-up's response is appended to the same tie-off file as
      a second section (`… (event-enforcement follow-up)`), so both turns
      are visible in the tie-off document
- [ ] `docs/release-notes.md` — bugfix entry: enforcement follow-up
      responses now recorded in the tie-off file (previously only the first
      malformed section was visible; follow-up events were dispatched but
      the turn itself was not persisted)

### Phase 3: Verification

- [ ] `cargo test` (full suite)
- [ ] `cargo clippy` — no new warnings
- [ ] Manual review of the resulting two-section tie-off shape against
      `FileSystemTieOffSink::append`'s header format
      (`## {knot} triggered by {label} {strand}` / `Timestamp:` /
      `session:` / `---` delimiter) — no new code path in the sink needed,
      the existing append machinery handles the existing file

Per AGENTS.md: **no live rig runs in this repo** — verification is via
`cargo test` / `cargo clippy` and the mock-CLI test harness only.

## Notes

- No domain/adapter change: `TieOff`, `TieOffSink`, and the sink's append
  behaviour are unchanged; this is a single missing persistence call in the
  application layer.
- Ordering is already correct: enforcement (and hence the new append) runs
  before the late event-file removal and the git-versioned commit, so the
  second section is committed together with the first.
- The second `KnotEventsMissing` (follow-up still incomplete) and the
  dispatch of follow-up events were already correct; only the document-level
  record was missing.
- Precedent: `write_tie_off` ("best-effort — `let _ = …append(tie_off)`") —
  the follow-up append follows the same non-fatal posture: a tie-off write
  failure must never fail the strand.
