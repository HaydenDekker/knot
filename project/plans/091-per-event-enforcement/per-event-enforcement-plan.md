# Plan 091: Per-Event Enforcement — Re-Ask When a Tie-Off Acknowledges Only Some Expected Events

## Related Plans

Extends [059 Tie-Off Event Enforcement](../059-tie-off-event-enforcement/tie-off-event-enforcement-plan.md),
replacing its zero-block gate (`tieoff_parser::has_no_events`) with a
per-event completeness check, and generalising the 059 follow-up prompt
from "your response did not contain any agent event blocks" to "your
response is missing these specific blocks". Reuses, unchanged in
behaviour, `extract_expected_event_ids` (the mirror of
`build_listener_context` added by 059), `inject_event_request` (059,
already a concise in-session follow-up per the [090
Concise In-Session Retry](../090-concise-in-session-retry/concise-in-session-retry-plan.md)
precedent), and the `occurred`-filter choke point in
`dispatch_events_to_consumers` (059 phase 5). The `TasksIncomplete`
self-continuation entry injected for water-marked aliases (plans
[086](../086-graceful-task-handoff/graceful-task-handoff-plan.md) /
[087](../087-event-source-continuation-and-queue-budget/087-event-source-continuation-and-queue-budget-plan.md))
is deliberately excluded from the completeness check — see D1.

## Related PRD

Extends the goals of [Tie-Off Event Enforcement](../prds/prd-tie-off-event-enforcement.md)
from "no event blocks at all" to "not all of the expected event blocks".

## Problem

Event acknowledgement is a per-event obligation: the injected `#
Subscriber Events` block instructs the producer to "emit exactly one
event block per subscriber event", each explicitly `occurred: true` or
`occurred: false`. But the service-side check is a **zero-block gate**:
`has_no_events()` only detects "did the agent emit *any* block at all".
Any block — even a set of pure `occurred: false` acknowledgements —
satisfies the gate, so a missing *individual* event is invisible to the
service and no re-ask happens.

**Evidence (rust-core-4 rig, 2026-09-20).** `prd-config-check`
(planning-loom) had a four-event subscriber list injected
(`PlanRequested`, `PlanReady`, `PlanComplete`, `ValidationSweep` — all
loom-level subscriptions). Its tie-off contained three `occurred: false`
acknowledgement blocks plus a prose mention of `PlanRequested` ("requested
an initial plan (PlanRequested)"). Service log:

```
event parse (knot=prd-config-check): 3 event(s) found — PlanReady=false, PlanComplete=false, ValidationSweep=false
```

The gate passed (3 blocks present), no `KnotEventsMissing`, no follow-up,
the knot completed normally, and nothing was dispatched —
`strategy-on-request`, the `PlanRequested` consumer, never fired. The
pipeline stalled silently at PRD intake; the user had to read the tie-off
to notice the missing event.

Two compounding causes:

1. **Agent side** — the agent treated its own producer event
   (`PlanRequested`) as a narrative action to describe, not a structured
   block to emit ("Now acknowledging the *remaining* subscriber events").
2. **Service side** — even with the agent-side slip, the zero-block gate
   cannot detect a partial acknowledgement and re-ask for the missing
   block. This plan fixes the service side, and adds one prompt line
   aimed at the agent-side slip.

## Target

The enforcement gate becomes a **per-event completeness check**:

- **Expected set** — `extract_expected_event_ids(knot, loom, all_knots)`:
  the same event IDs as the injected subscriber list, minus the
  `TasksIncomplete` self-continuation entry (which that function never
  includes). An empty expected set means no enforcement (as today when no
  consumers subscribe).
- **Actual set** — the `event_id`s parsed from the tie-off by
  `extract_agent_events`.
- **Missing** — `expected − actual` (one-way subset check: extra blocks
  the agent emits that nobody subscribes to never block enforcement).
  Non-empty `missing` triggers enforcement — the zero-block case is the
  special case `missing = expected` (identical behaviour to today, plus
  an accurate `missing_events` in the log).
- **`event: None` stays a blanket acknowledgement** (plan-059 contract):
  if the parsed set contains `None`, the check passes, whatever else is
  missing.
- **Log the missing set** — `LoomEvent::KnotEventsMissing` gains a
  `missing_events: Vec<String>` field; `expected_events` keeps carrying
  the *full* expected set (unchanged). The service-log line renders
  `expected=... missing=...`; the system-event payload gains
  `missing-events`.
- **Name the missing blocks in the follow-up** — `inject_event_request`
  takes the missing IDs and its prompt says which blocks are missing,
  re-sending the full listener context (format + rules) unchanged.
- **Second attempt** — `missing` is recomputed from the follow-up
  response. Any events found in it are dispatched (existing path; the
  `occurred: false` filter inside `dispatch_events_to_consumers` is
  untouched). If `missing` is still non-empty, a second
  `KnotEventsMissing` is logged with the still-missing IDs; no further
  retry (059 bound).
- **Prompt reinforcement** — one new rule line in
  `build_listener_context`: mentioning an event in narrative text is not
  an acknowledgement; only structured ```markdown blocks are parsed;
  emit a block for every listed event, even when `occurred: false`.
- **Best-effort semantics unchanged** — no session ID → log only; at most
  one follow-up; enforcement never fails the strand.

Non-goals:

- No change to which events enter the injected subscriber list
  (`build_listener_context`'s matching logic is untouched; only its
  Rules text gains one line).
- No change to dispatch filtering — `occurred: false` events are never
  dispatched (059 phase-5 choke-point rule).
- No change to tie-off format or event-block parsing.
- `has_no_events` is kept and stays exercised: it is the fast path for
  the zero-block case (skip the full parse; `missing = expected`).
- No project document format change → no `knot-update` migration entry.

## Existing Tests

| Test | File | What it covers | Status after this plan |
|------|------|----------------|------------------------|
| `has_no_events_*` (5 tests) | `src/domain/tieoff_parser.rs` | Zero-block gate | Unchanged — function kept (fast path) |
| `extract_agent_events_*` (8 tests) | `src/domain/tieoff_parser.rs` | Block parsing incl. `occurred` | Unchanged |
| `KnotEventsMissing` serialisation/fields (2 tests) | `src/domain/events.rs` (~line 2624) | Variant shape | **Must extend** — new `missing_events` field in the fixtures |
| `build_listener_context` tests | `src/domain/events.rs` | Injected subscriber-list text | **Must extend** — new rule line asserted |
| `render_loom_event_line` `KnotEventsMissing` case (~line 935) | `src/adapters/service_log.rs` | Log-line shape | **Must change** — line now includes `missing=...` |
| `inject_event_request_success_with_events` | `src/application/session_resume.rs` | Follow-up returns response | **Must change** — new `missing_events` param |
| `inject_event_request_no_session_id_returns_err` | `src/application/session_resume.rs` | No session → Err | **Must change** — signature |
| `inject_event_request_runner_error_propagates` | `src/application/session_resume.rs` | Error propagation | **Must change** — signature |
| `inject_event_request_repeats_listener_context` | `src/application/session_resume.rs` | Prompt repeats listener context; asserts old preamble "did not contain any agent event blocks" | **Must change** — preamble is replaced; assert the missing IDs are named + listener context still repeated |
| `inject_event_request_does_not_resend_profile_prompt` | `src/application/session_resume.rs` | Concise follow-up (empty profile prompt) | **Must change** — signature only |
| `process_strand_enforcement_no_consumers_skipped` | `src/application/usecases/process_strand.rs` (`event_enforcement_tests`) | No expected events → no enforcement | Passes as-is (regression guard for the empty-expected-set guard) |
| `process_strand_enforcement_events_present_skipped` | same | Full expected set emitted (1 of 1) → skipped | Passes as-is — the emitted block is the complete expected set |
| `process_strand_enforcement_occurred_false_skipped` | same | Full set with `occurred: false` → skipped | Passes as-is — false acks satisfy completeness |
| `process_strand_enforcement_missing_events_logs_and_retries` | same | Zero blocks → `KnotEventsMissing` + follow-up | **Must extend** — zero-block special case; assert `missing_events == expected` |
| `process_strand_enforcement_followup_produces_events_dispatched` | same | Follow-up events dispatched | Passes as-is (follow-up emits the complete set) |
| `process_strand_enforcement_followup_still_missing_logs_twice` | same | Follow-up empty → second log | Passes as-is; **extend** with still-missing assertion |
| `process_strand_enforcement_no_session_id_log_only` | same | Stdio → log only | **Must change** — `missing_events` in the logged variant |
| `regression_event_enforcement_flow_still_works` (~line 7681) | `src/application/usecases/process_strand.rs` | 059 flow end-to-end | Passes as-is (asserts presence, not shape) |
| `test_event_enforcement_with_real_pi`, `_stdio_no_reentry`, `_followup_also_fails`, `_multiple_consumers`, `_regression_basic_pipeline`, `_regression_normal_dispatch`, `_followup_occurred_false_not_dispatched` | `tests/event_enforcement.rs` | Integration: full flows, 059 phase-5 guard | Pass as-is (all use the zero-block or full-set shapes) |
| `test_event_enforcement_event_none_passes` | `tests/event_enforcement.rs` | `event: None` → no enforcement | **Unchanged — the D2 regression guard** |
| `test_event_enforcement_no_consumers` | `tests/event_enforcement.rs` | No consumers → skipped | Passes as-is |

## Test Gaps

- No test that a tie-off acknowledging **only some** expected events
  triggers enforcement with the correct `missing` set (the bug this plan
  fixes).
- No test that a follow-up supplying the missing blocks is dispatched and
  produces exactly **one** `KnotEventsMissing`.
- No test that a follow-up still short of the expected set logs a second
  `KnotEventsMissing` carrying the still-missing IDs.
- No test that `event: None` alongside a partial set still passes
  (blanket ack wins).
- No test that extra, non-subscribed event blocks never block
  enforcement (one-way check).
- No test that the follow-up prompt names the missing event IDs.
- No test that the `KnotEventsMissing` service-log line carries
  `missing=...`.

## Phases

### Phase 0: Failing Tests

**`src/domain/tieoff_parser.rs`** — unit tests for the new helper
`missing_event_ids` (signature and semantics in Phase 1.1):

1. `missing_event_ids_all_present_returns_empty()` — expected
   `[PlanCreated, PlanRejected]`, tie-off has both blocks → `[]`.
2. `missing_event_ids_partial_returns_missing_sorted()` — expected
   `[PlanCreated, PlanRejected]`, tie-off has only `PlanCreated`
   (plus prose mentioning `PlanRejected`) → `[PlanRejected]`. (Prose
   must not count — the rust-core-4 case.)
3. `missing_event_ids_zero_blocks_returns_all_expected()` — expected
   `[A, B]`, body only → `[A, B]` (sorted).
4. `missing_event_ids_event_none_is_blanket_ack()` — expected
   `[A, B]`, tie-off has only `event: None` → `[]`.
5. `missing_event_ids_event_none_wins_over_partial()` — expected
   `[A, B]`, tie-off has `A` + `None` → `[]`.
6. `missing_event_ids_extra_events_ignored()` — expected `[A]`, tie-off
   has `A` + `MyOwnEvent` → `[]`.
7. `missing_event_ids_duplicates_deduped()` — expected `[A]`, tie-off
   has `A` twice → `[]`.
8. `missing_event_ids_occurred_false_counts_as_present()` — expected
   `[A]`, tie-off has `A` with `occurred: false` → `[]` (acknowledgement
   is present; the `occurred` value is dispatch's concern, not the
   gate's).

**`src/application/session_resume.rs`**:

9. `inject_event_request_prompt_names_missing_events()` — `expected`
   listener context listing `PhaseReady` + `ImplementationNote`,
   `missing_events = ["ImplementationNote"]` → prompt contains
   `ImplementationNote` in the missing list, contains the listener
   context (`## Agent Events` / `PhaseReady`), and does **not** contain
   the old "did not contain any agent event blocks" preamble.
10. `inject_event_request_prompt_zero_block_case_lists_all()` —
    `missing_events` = full expected set → all IDs named.

**`src/application/usecases/process_strand.rs`** — new tests in
`event_enforcement_tests` (fixtures: `build_consumer_knot` ×2 for two
expected events, `build_enforcement_strand`, `event_block` /
`event_occurred_false_block`):

11. `process_strand_enforcement_partial_missing_triggers_followup()` —
    producer knot with two subscriber events (`PlanCreated`,
    `PlanRejected`); first response carries only the `PlanCreated` block
    (session ID set); second response carries the `PlanRejected` block.
    → exactly one `KnotEventsMissing`, its `missing_events ==
    [PlanRejected]` and `expected_events` carries both; follow-up was
    invoked (runner saw 2 executions, second with `--session-id`); the
    `PlanRejected` dispatch is recorded.
12. `process_strand_enforcement_followup_still_partial_logs_second()` —
    same setup; second response carries an *unrelated* block
    (`SomethingElse`) only → two `KnotEventsMissing`; the second's
    `missing_events == [PlanRejected]`; runner saw exactly 2 executions
    (no third); the unrelated block is dispatched.
13. `process_strand_enforcement_event_none_blanket_ack_passes()` —
    expected `[PlanCreated]`; response carries only `event: None` → no
    `KnotEventsMissing`, no follow-up (unit-level D2 guard alongside the
    integration test).
14. `process_strand_enforcement_extra_events_beyond_expected_pass()` —
    expected `[PlanCreated]`; response carries `PlanCreated` +
    `MyOwnEvent` blocks → no enforcement.
15. Extend `process_strand_enforcement_missing_events_logs_and_retries`
    with a `missing_events == expected` assertion (zero-block special
    case), and `process_strand_enforcement_followup_still_missing_logs_twice`
    with the still-missing assertion.

**`src/adapters/service_log.rs`**:

16. Extend the `KnotEventsMissing` `render_loom_event_line` test: the
    line contains `expected=PlanCreated,PlanRejected missing=PlanRejected`.

### Phase 1: Implementation

**1.1 `src/domain/tieoff_parser.rs` — the completeness helper.**

```rust
/// Per-event completeness check (plan 091).
///
/// Returns the expected event IDs that have **no** corresponding
/// structured block in `content`, sorted. One-way subset check: blocks
/// for events not in `expected` are ignored. A parsed `event: None`
/// block is a blanket acknowledgement (plan 059) — it satisfies the
/// check regardless of anything else. `occurred: false` blocks count as
/// present (acknowledgement is the gate's concern; dispatch filters
/// `occurred` itself).
pub fn missing_event_ids(expected: &[String], content: &str) -> Vec<String> {
    let events = extract_agent_events(content);
    if events.iter().any(|e| e.event_id == "None") {
        return Vec::new();
    }
    let actual: std::collections::HashSet<&str> =
        events.iter().map(|e| e.event_id.as_str()).collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|id| !actual.contains(id.as_str()))
        .cloned()
        .collect();
    // (sorted — same order as extract_expected_event_ids)
    missing
}
```

`has_no_events` is kept unchanged (fast path + its tests).

**1.2 `src/domain/events.rs` — `missing_events` on the log variant.**

```rust
KnotEventsMissing {
    loom_id: LoomId,
    knot_id: KnotId,
    strand_path: StrandPath,
    /// All events the agent was expected to acknowledge (unchanged).
    expected_events: Vec<String>,
    /// The subset with no block in the tie-off (plan 091). Equals
    /// `expected_events` in the zero-block case.
    missing_events: Vec<String>,
    timestamp: String,
},
```

Source-compatible for existing `{ loom_id, .. }` match arms
(`activity.rs`, tests).

**1.3 `src/application/session_resume.rs` — name the missing blocks.**

`inject_event_request` gains `missing_events: Vec<String>` (replacing the
blanket wording; position after `listener_context`). The prompt becomes:

```
Your previous response did not acknowledge the following required
event blocks: <missing, comma-separated>.

Please emit exactly one event block for each of them (occurred: true or
false, as appropriate), in the format instructed below:

<listener context unchanged>
```

Everything else (no profile prompt, `--session-id`, strand path,
timeout) is untouched.

**1.4 `src/application/usecases/process_strand_helpers.rs` — the gate.**

Replace the `has_no_events`-only gate inside the existing
`if !resolved.listener_context.is_empty()` block with:

```rust
let expected = extract_expected_event_ids(knot, loom_id, &resolved.all_knots);
// Empty expected set: only self-continuation (TasksIncomplete) was
// injected — a conditional signal, never enforced (plan 091 D1).
if !expected.is_empty() {
    if let Some(ref content) = outcome.tie_off_content() {
        let missing = if crate::domain::tieoff_parser::has_no_events(content) {
            expected.clone()          // fast path: zero blocks
        } else {
            crate::domain::tieoff_parser::missing_event_ids(&expected, content)
        };
        if !missing.is_empty() {
            // KnotEventsMissing { expected_events: expected.clone(),
            //                     missing_events: missing.clone(), ... }
            // emit_system: payload gains ("missing-events", Some(missing.join(", ")))
            // message: "completed but did not acknowledge all expected events (missing: …)"
            // follow-up: inject_event_request(..., listener_context, missing.clone(), ...)
            // on Ok(response):
            //     followup_events = extract_agent_events(&response)
            //     dispatch (unchanged call; occurred filter inside the dispatcher)
            //     still_missing = missing_event_ids(&missing, &response)
            //     — the follow-up was asked to emit blocks for the missing
            //       set only, so the re-check diffs against that set (an
            //       empty follow-up leaves everything missing)
            //     if !still_missing.is_empty() → second KnotEventsMissing
            //       { expected_events: expected, missing_events: still_missing }
        }
    }
}
```

Notes:

- The zero-block case is a strict generalisation of today's behaviour —
  same trigger, same follow-up, plus an accurate `missing_events`.
- Second-attempt semantics tighten slightly: today any non-empty
  follow-up suppresses the second log; now the second log fires when the
  follow-up's blocks do not cover what was missing (a follow-up emitting
  only an unrelated block no longer counts as an acknowledgement). This
  is the point of the plan, and the dispatch of whatever the follow-up
  did emit is unchanged. The re-check diffs the follow-up against the
  *missing* set (not the full expected set) — the follow-up prompt asks
  for the missing blocks only, and blocks already delivered on the main
  path are not re-required.
- All existing call sites of `emit_system` for `KnotEventsMissing` move
  with the new shape; the first-attempt message text changes from "emitted
  no expected events" to "did not acknowledge all expected events
  (missing: …)" so the service log is accurate in both cases.

**1.5 `src/domain/events.rs` — `build_listener_context` rule line.**

Append to the Rules list (after the "When `occurred: false`" rule):

```
- Mentioning an event in narrative text is not an acknowledgement — only
  structured ```markdown event blocks are parsed. Emit a block for every
  event listed above, even when `occurred: false`.
```

**1.6 `src/adapters/service_log.rs` — log line.**

```rust
LoomEvent::KnotEventsMissing { loom_id, knot_id, strand_path,
                               expected_events, missing_events, .. } =>
    format!("KnotEventsMissing loom={} knot={} strand={} expected={} missing={}",
            ..., expected_events.join(","), missing_events.join(","))
```

### Phase 2: Verify

- `cargo test` — full suite green; watch:
  - the Phase 0 tests (partial-missing, blanket-ack, one-way, prompt
    naming, log line),
  - the untouched 059 regression guards
    (`test_event_enforcement_event_none_passes`,
    `test_event_enforcement_followup_occurred_false_not_dispatched`,
    `process_strand_enforcement_events_present_skipped` /
    `occurred_false_skipped`),
  - `build_listener_context` tests (new rule line),
  - `tests/event_enforcement.rs` integration suite.
- `cargo clippy --all-targets` clean.

### Phase 3: Docs + Version

1. `docs/concepts.md` — event-enforcement section (the
   `KnotEventsMissing` documentation around line 591): enforcement
   triggers when any expected event lacks a block (not only when the
   tie-off has zero blocks); `event: None` remains a blanket
   acknowledgement; `TasksIncomplete` is not enforced.
2. `.agents/skills/knot-init/knot-glossary.md` — *Event Enforcement*
   entry: "zero event blocks" → "any expected event without a
   structured block (including the zero-block case)".
3. `docs/release-notes.md` — v0.49.0 entry (fix + improvement):
   per-event enforcement — the follow-up re-ask now fires when a
   tie-off acknowledges only some of the expected events, names the
   missing blocks, and logs `missing=...` alongside `expected=...`.
4. `knot-update` skill: **no migration entry** (no document-format
   change; the rig's knot/loom/profile files are untouched).
5. Bump `Cargo.toml` to `0.49.0`; `cargo install --path .`.

## Notes

- **D1 — `TasksIncomplete` is excluded by construction.** It is a
  conditional self-continuation signal (emitted only while tasks
  remain), not a subscriber acknowledgement. `extract_expected_event_ids`
  never includes it, so a water-marked knot that finishes its tasks
  without a `TasksIncomplete` block is not re-asked. The empty-expected
  guard (1.4) also covers the edge where *only* `TasksIncomplete` was
  injected: listener context non-empty, expected set empty → no
  enforcement.
- **D2 — `event: None` stays a blanket ack.** Plan 059's contract
  ("explicit no-events declaration → no enforcement") is preserved,
  including alongside a partial set (test 5). The rig's `knot-design` /
  `knot-dispatch` guidance on `event: None` is unaffected.
- **D3 — one-way subset check.** The check never demands more than the
  subscriber list. A knot emitting its own producer events that nobody
  subscribes to (common: a knot's `event-description` event with no
  `event:` subscriber yet) is never penalised, and an agent that
  *over-acknowledges* is fine.
- **Why not filter the listener context to the missing events in the
  follow-up?** The context is a single pre-built string (event list +
  format example + rules); rebuilding it per-missing-set duplicates
  `build_listener_context`'s logic and costs nothing saved in practice
  (the context is short, and the agent benefits from re-seeing the full
  format example). Naming the missing IDs in the preamble is the
  high-signal part.
- **Performance.** The tie-off content is parsed twice in the
  enforcement path (`dispatch_agent_events` already parsed it for
  dispatch; `missing_event_ids` parses again) — plus the fast path skips
  parsing entirely in the zero-block case. Tie-offs are small (a few KB);
  the cost is negligible. A shared-parse refactor is deliberately not
  done: `dispatch_agent_events` and the helper live in different
  functions and the parse is not on a hot path.
- **The agent-side slip is addressed, but the service-side gap was the
  load-bearing bug.** The new rule line (1.5) reduces recurrence of the
  "narrated instead of emitted" mode; even if the agent still slips,
  the completeness gate now re-asks once with the exact missing list —
  the rust-core-4 case would have produced a `PlanRequested` block via
  the follow-up, or at minimum a `KnotEventsMissing` service-log line
  naming it.
- **Immediate recovery for the stuck rust-core-4 rig** (out of scope for
  this plan, recorded here for traceability): re-touch
  `project/prds/prd-todo-list.md` to re-run `prd-config-check`, or drop
  an event file into `tie-offs/rig/strategy-loom/PlanRequested/`.
