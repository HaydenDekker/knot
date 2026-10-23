# Phase 3: Commit trailers and structured trace fields

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 2 is green and committed.**

## Checklist
- [ ] `FileSystemGitVersioner::commit` (`src/adapters/outbound/git_versioner.rs`)
      appends the trailer block to the commit message — last paragraph,
      git's own trailer convention (`session:`, `event:`, `source:`,
      `original_strand:` as `Token: value` lines); the subject and the
      1000-line body truncation are unchanged; fields that are `None`
      (e.g. `original_strand` for a loom- or rig-scoped system event)
      are omitted, not written empty; the commit signature gains the
      trace fields (they arrive from the delivery for `EventDelivered`
      strands — Phase 2 — and from the run context for file-triggered
      strands)
- [ ] Trailer tests — the subject is unchanged; the trailer block is
      present and parseable by `git interpret-trailers`; `None` fields
      are absent; one test per field so a dropped field fails one test
- [ ] The service-log line grammar gains the trace fields on the
      run-lifecycle lines — `session=`, `event=`, `source=`,
      `original-strand=` in `render_loom_event_line`
      (`src/adapters/service_log.rs`); the single-line invariant holds
      (no spaces inside values, `None` omitted)
- [ ] Extend `tests/consolidated_log.rs` for the new grammar fields
      (assert the fields appear on the run-lifecycle lines and the
      line stays single-line greppable)
- [ ] Verification — clippy clean for the new code; the trailer tests
      green + `cargo test --test consolidated_log` green; commit sha
      recorded as this phase's external-verdict request. The ON-COMMIT
      suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- The trace fields are already carried end-to-end after Phase 2
  (delivery → pipeline); this phase only makes them durable (the
  commit) and greppable (the service log) — no new plumbing upstream of
  the commit path.
