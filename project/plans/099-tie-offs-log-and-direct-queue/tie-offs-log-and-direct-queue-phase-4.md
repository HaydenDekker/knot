# Phase 4: The unified `tie-offs.log`

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 3 is green and committed.**

## Checklist
- [x] `ProcessStrand::compute_tie_off_path`
      (`src/application/usecases/process_strand.rs`) returns the single
      `tie-offs/<rig>/tie-offs.log` (the per-(loom, knot) path retires);
      the `TieOffSink` port (`src/application/ports.rs`) is reworded to
      an append-to-one-log contract
- [x] `FileSystemTieOffSink::append`
      (`src/adapters/outbound/tieoff_sink.rs`) appends the section to
      the unified log: the trigger header reworked into a structured
      first line (knot, event, strand, timestamp, session, source,
      original strand — one greppable line), then the content; the
      event audit blocks slimmed to id, occurred, reason, and
      timestamp — no payload bodies (the body stays in the commit per
      D7; the full session narrative stays reachable via
      `pi --session-id`)
- [x] Sole-writer serialisation — appends from concurrent knots are
      serialised (one writer / locked append path); the service is the
      only writer of the file
- [x] `TieOffSink::read_content` re-targets the unified log — the
      Deleted-event history path (`extract_last_n`, last 5 sections for
      the same strand) and the enforcement follow-up both read and
      append the one file
- [x] Re-target `tests/tie_off.rs` to the unified log (section format
      against one accumulating file, not per-knot files)
- [x] Re-target `tests/event_enforcement.rs` — the enforcement
      follow-up append lands in `tie-offs.log`, the same file the
      original section was appended to
- [x] Add to `tests/consolidated_log.rs`: the section grammar (the
      structured first line), sole-writer serialisation under
      concurrent appends (two knots appending at once — both sections
      intact, no interleaving), and the enforcement follow-up appending
      to the same file
- [x] Verification — clippy clean for the new code;
      `cargo test --test consolidated_log` + `cargo test --test
      event_enforcement` + `cargo test --test tie_off` green; commit sha
      recorded as this phase's external-verdict request. The ON-COMMIT
      suite is not required by this item.
      **Commit sha: `596ca82`** (full suite green: 1533+ passed,
      0 failed; clippy clean — warnings only)

## Deviations
- The structured header carries an extra optional field, `event-id=`,
  between `event=` and `strand=`. The plan's seven-field list folded
  the old header's two event notions together; the run's own event
  label (Created/Modified/Deleted) stays in `event=` and the
  triggering emission's id (the old `event:` metadata line) rides
  `event-id=` so nothing is lost from the greppable line.
- `tests/event_enforcement.rs` needed no re-target: it asserts the
  event flow (KnotEventsMissing → follow-up → EventsDispatched), not
  tie-off file paths. The follow-up append goes through the same
  `TieOffSink::append` — the single append path *is* the same-file
  guarantee, covered by the sink unit tests and the Phase 4
  consolidated_log tests.
- The concurrent-appends test at the binary level uses two looms
  watching one strand dir; true thread-level racing is covered
  deterministically by the sink unit test
  `append_serialises_concurrent_writers`.

## Discoveries
- The file watcher reports `Modified` (not `Created`) for freshly
  written strand files in the binary-level tests — the section grammar
  regexes accept both.

## Notes
- After this phase, `tie-offs/<rig>/` holds exactly: `state.json`,
  `knot-service.log`, `tie-offs.log`, and `events/` — the plan's
  target layout, minus the cutover diagnostics (Phase 5).
- The read norm (logs — query, never whole-file read; `state.json` for
  snapshots; git is the rotation) is normative but travels with the
  skills repointing, not this phase's file set.
