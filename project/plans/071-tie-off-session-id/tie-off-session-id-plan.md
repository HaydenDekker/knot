# Plan: Record the Pi Session ID in Tie-Off Sections

## Problem

Every strand processing invokes a pi session, and the session ID is already
captured by the JSON adapter: `PiJsonAgentRunner` parses the first
`{"type":"session","id":...}` JSON-L line and carries the ID in
`AgentOutput.metadata.session_id` (success) or `PortError.session_id`
(failure). The session-resume retry loop
(`src/application/session_resume.rs`) tracks it into
`ResolvedExecution.session_id` — where it is used only for event-enforcement
follow-up re-entry.

After processing, the ID is **thrown away**. The tie-off section — the durable,
human-readable record of what happened to a strand — records *who* (knot),
*what* (event type), *where* (strand path), *when* (timestamp), and *why*
(event metadata), but not *which pi session* produced the output. Consequences:

1. **No traceability from tie-off to session.** A reviewer reading a tie-off
   section cannot resume or inspect the pi session that produced it
   (`pi --session-id <id>` / `~/.pi/agent/sessions/`). The `--name` title
   (Plan 32) helps find the session in a listing, but titles are not unique
   and are not a reliable handle.
2. **The loom-log only records retries, not the primary session.**
   `LoomEvent::SessionResumed` fires only when the resume-retry loop re-enters
   a session — the session ID of a clean first-attempt run appears nowhere.
3. **Debugging dead ends.** When a knot produces bad output or a follow-up
   fails, the operator has no way to open the exact conversation that produced
   the tie-off.

## Target

When this plan is complete:

1. **Tie-off sections carry the session ID.** Every tie-off section written by
   `FileSystemTieOffSink::append` includes an optional metadata line, placed
   after `Timestamp:` and before the event-trigger metadata lines:

   ```markdown
   ## review triggered by Modified strand.md
   Timestamp: 2026-08-22T12:00:00+00:00
   session: 1f2e3d4c-...            ← new — present when a session ID was captured
   event: PlanCreated                ← existing event-trigger metadata (unchanged)
   source: producer-knot
   original_strand: ...
   ---
   Agent output...
   ```

   The line is **omitted entirely** when no session ID is known (stdio adapter,
   unparseable output) — old sections and adapter-less sections keep their
   exact current shape, so existing files parse and read identically.

2. **All tie-off write paths are covered.** The session ID is threaded from
   execution to the `TieOff` entity on every path that writes a section:
   success (from `ResolvedExecution.session_id`), failure (from
   `PortError::session_id()`), and the pre-resolution error branch in
   `ProcessStrand::execute` (from the same `PortError`). Timeout outcomes
   already skip tie-off writing (section preserved unchanged) — no change.
   The event-enforcement follow-up re-enters the *same* session
   (`--session-id`), so the primary ID recorded remains correct; the
   session-resume retry loop likewise preserves one session ID across
   attempts.

3. **The `TieOff` entity models the field.** `TieOff.session_id:
   Option<String>` with `#[serde(default, skip_serializing_if =
   "Option::is_none")]` — backward-compatible with any serialized tie-off
   state.

4. **Parser behaviour is deliberately unchanged.** `tieoff_parser::
   parse_sections` special-cases only the header and `Timestamp:` lines;
   the existing event metadata lines (`event:`, `source:`,
   `original_strand:`) already flow into the parsed section **body**.
   `session:` joins that same body-metadata category — no parser change,
   consistent with the established precedent. (Structured parsing of the
   line is an open question in Notes.)

## Design Notes (Hexagonal)

- **Domain** — `TieOff` (`src/domain/entities.rs:231`) gains
  `session_id: Option<String>` (serde `default` + `skip_serializing_if`).
  `TieOffSection` (`src/domain/tieoff_parser.rs:10`) is **not** changed
  (see Target 4). No domain rule changes — the field is carried, not
  interpreted.
- **Application — use case** — `process_strand_helpers::write_tie_off`
  (`src/application/usecases/process_strand_helpers.rs:40`) gains a
  `session_id: &Option<String>` parameter and sets `TieOff.session_id`.
  `ProcessStrand::execute` (`src/application/usecases/process_strand.rs`)
  passes `&resolved.session_id` on the normal path, and in the
  `resolve_config_and_build` error branch extracts
  `err.session_id().cloned()` **before** `return Err(err)` to populate the
  failure tie-off. `ResolvedExecution` already carries the ID — no new
  plumbing into the helper's existing call shape beyond one parameter.
- **Outbound adapter** — `FileSystemTieOffSink::append`
  (`src/adapters/outbound/tieoff_sink.rs`) emits `session: {id}\n` when
  `tie_off.session_id` is `Some`. `write()` (overwrite mode) writes raw
  content and is unaffected. `MockTieOffSink` / `TrackingTieOffSink`
  (`src/application/usecases/test_fixtures.rs`) record the whole `TieOff`
  — no behaviour change, but the recorded value becomes assertable.
- **Inbound adapters** — no changes. `state.json` does not carry tie-offs;
  the loom-log and rig-log are untouched (see Out of scope).

## Existing Tests

| Test | What it covers | Status |
|---|---|---|
| `tieoff_sink.rs` — `append_mode_creates_file`, `append_mode_adds_section`, `append_mode_preserves_history` | Append header format (`## … triggered by …`, `Timestamp:`, `---`), section ordering, history preservation | ✅ Green — pins current header shape (no session line) |
| `tieoff_sink.rs` — `tieoff_write_new_file`, `tieoff_overwrite_existing`, `tieoff_create_parent_dirs`, `tieoff_sink_trait_object_safe` | Overwrite-mode `write()`, path handling, object safety | ✅ Green — construct `TieOff` literals (need the new field) |
| `entities.rs` — `TieOff` serde round-trip tests (~line 727–790) | Serialize/deserialize of `TieOff` incl. `Failed` status | ✅ Green — extend for `session_id` |
| `tests/tie_off.rs` — path structure + append-history tests | `ProcessStrand` → `TrackingTieOffSink` appends, path layout, section content | ✅ Green — appends gain a new field |
| `tests/pipeline.rs` | Full `ProcessStrand` flow with mocked ports | ✅ Green |
| `process_strand.rs` unit tests (incl. ~line 5297 session-resume tests) | `TrackingTieOffSink` appends across success/failure/resume paths | ✅ Green — construct/consume `TieOff` |
| `pi_json.rs` — `test_json_runner_parses_session_id`, `…_timeout_captures_session_id`, `…_nonzero_exit_captures_session_id` | Session ID extraction from JSON-L on success, timeout, non-zero exit | ✅ Green — source of the ID already works |
| `tests/session_resume.rs` | Retry loop, `SessionResumed` events, follow-up re-entry | ✅ Green — ID tracking this plan reuses |

## Test Gaps

- No test asserts the `session:` line is **emitted** when
  `TieOff.session_id` is `Some` (position: after `Timestamp:`, before event
  metadata).
- No test asserts the line is **omitted** when `session_id` is `None`
  (byte-identical output to today's shape).
- No test that `write_tie_off` / `execute` **thread** the ID: success output
  with `metadata.session_id` → append carries it; failure `PortError` with a
  session ID → failure append carries it; no ID anywhere → `None`.
- No serde round-trip test for `TieOff.session_id` (presence, absence,
  backward compat: old JSON without the field deserialises to `None`).
- No acceptance test with the **real** `FileSystemTieOffSink`: a mock runner
  returning `AgentOutput { metadata: Some(…session_id…) }` produces an
  on-disk tie-off file containing `session: <id>`; a stdio-style runner
  (`metadata: None`) produces a file without the line. `tests/helpers.rs`
  `ProcessStrandBuilder` currently always wires `TrackingTieOffSink` — it
  needs a `with_real_tie_off_sink(rig_dir)` option (precedent:
  `with_real_event_dispatcher` from Plan 070).

## Phases

### Phase 0: Domain — `TieOff.session_id` (TDD)

Failing tests first: serde round-trip with `session_id` set; round-trip with
`None` asserting the field is **absent from JSON** (`skip_serializing_if`);
deserialising legacy JSON (no field) yields `None`. Then add the field to
`TieOff` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
Fix the compile fallout across `TieOff` literal constructors
(`tieoff_sink.rs` tests, `process_strand.rs`, `entities.rs` tests) with
`session_id: None` — behaviour unchanged, all existing tests green.

### Phase 1: Adapter — sink emits the line (TDD)

Failing tests in `tieoff_sink.rs`:

- `append` with `session_id: Some("sess-123")` writes `session: sess-123`
  on the line **after** `Timestamp:` and before any `event:`/`source:`/
  `original_strand:` lines.
- `append` with `session_id: None` produces byte-identical output to the
  pre-change format (existing `append_mode_*` tests stay green unmodified).
- Combined case: session line **and** event metadata both present, correct
  order.

Implement in `FileSystemTieOffSink::append`.

### Phase 2: Use-case threading (TDD)

Extend `write_tie_off` with `session_id: &Option<String>`; update the call in
`ProcessStrand::execute` to pass `&resolved.session_id`. In the
`resolve_config_and_build` error branch, capture
`let session_id = err.session_id().cloned();` before constructing the
failure `TieOff`. Failing tests first (via `TrackingTieOffSink` in
`process_strand.rs` / `tests/tie_off.rs`):

- success runner returning `metadata: Some(AgentInvocationMetadata {
  session_id: Some("sess-a"), … })` → recorded append has
  `session_id == Some("sess-a")`;
- failing runner with `PortError::Timeout { session_id: Some("sess-b"), … }`
  → recorded failure append has `session_id == Some("sess-b")`;
- runner with `metadata: None` (stdio style) → append has `session_id: None`;
- resume path (first attempt fails with session ID, retry succeeds) → append
  carries the single session ID.

### Phase 3: Acceptance test — real sink, on-disk file

Add `ProcessStrandBuilder::with_real_tie_off_sink(rig_dir)` in
`tests/helpers.rs` (wiring `FileSystemTieOffSink` instead of the tracking
sink, mirroring `with_real_event_dispatcher`). Acceptance test in
`tests/tie_off.rs`:

1. Mock runner returns `AgentOutput` with `metadata.session_id =
   Some("accept-sess")` → execute a `Created` strand → read the on-disk file
   `tie-offs/<rig>/<loom>/tie-off-<knot>.md` → assert it contains
   `session: accept-sess` between `Timestamp:` and `---`.
2. Repeat with `metadata: None` → file contains no `session:` line and its
   header block matches today's exact shape.
3. Append a second event to the same tie-off → both sections parse via
   `tieoff_parser::parse_sections` (session line lands in the body, header
   and timestamp still structured) — pins the "parser unchanged" decision.

### Phase 4: Documentation

- `knot-manage` skill (Tie-Off File Format section) and `knot-dispatch`
  skill (tie-off example): add the optional `session:` line to the format
  description with a "present when the pi session ID was captured" note.
- `knot-update` changelog: entry for the new binary version recording the
  tie-off section format gain (optional line, no migration — old files
  unchanged, new readers tolerate its absence).
- Install updated skills globally per AGENTS.md and verify with `diff`.

## Notes

- **Line placement.** After `Timestamp:`, before the event-trigger
  metadata. `session:` describes *this execution* (same category as the
  timestamp); `event:`/`source:`/`original_strand:` describe the *trigger*
  (event-file routing). Keeping the two groups distinct makes the header
  self-explanatory.
- **Lowercase `session:`** matches the other body-metadata lines
  (`event:`, `source:`, `original_strand:`) and stays clearly apart from
  `Timestamp:`, the one line `parse_sections` special-cases.
- **Parser alternative (decide in Phase 3, record here if changed).**
  Adding `TieOffSection.session_id: Option<String>` parsed by
  `parse_sections` would give structured access for future consumers (e.g.
  knot-inspect tooling) and keep the line out of `deleted_prompt` history
  bodies. Rejected for now: the existing event metadata lines already live
  in the body, the Deleted-event history prompt is fine showing the line,
  and no consumer needs it structured yet. If one appears, it is a small,
  backward-compatible follow-up.
- **Why the tie-off and not the loom-log.** The tie-off is the per-strand,
  append-only, human-readable record — exactly where a reviewer looks when
  auditing a section. The loom-log is a machine event stream; adding
  `KnotCompleted.session_id` there is a separate, optional concern and is
  out of scope. (The `SessionResumed` precedent shows the loom-log carries
  session context only for exceptional paths.)
- **Stdio adapter.** `PiStdioAgentRunner` produces no metadata — the line
  is simply absent. No adapter change required.
- **Backward compatibility.** Old tie-off files (no line) are untouched and
  parse identically; new files with the line parse with the line in the
  body, same as today's event metadata lines. `TieOff` serde is
  `default`-tolerant both directions.
- **Out of scope.** Session ID in loom-log/rig-log events, `state.json`,
  git commit subjects, or the `--name` title; structured `TieOffSection`
  parsing; any change to session-resume or event-enforcement behaviour.
- **Binary version.** The format gain ships with the next version bump via
  plan completion; the `knot-update` changelog (Phase 4) is the record.
