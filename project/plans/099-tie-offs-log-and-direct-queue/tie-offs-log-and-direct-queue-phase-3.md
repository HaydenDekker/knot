# Phase 3: Commit trailers and structured trace fields

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 2 is green and committed.**

## Checklist
- [x] `FileSystemGitVersioner::commit` (`src/adapters/outbound/git_versioner.rs`)
      appends the trailer block to the commit message — last paragraph,
      git's own trailer convention (`session:`, `event:`, `source:`,
      `original_strand:` as `Token: value` lines); the subject and the
      1000-line body truncation are unchanged; fields that are `None`
      (e.g. `original_strand` for a loom- or rig-scoped system event)
      are omitted, not written empty; the commit signature gains the
      trace fields (they arrive from the delivery for `EventDelivered`
      strands — Phase 2 — and from the run context for file-triggered
      strands). The trace travels as a `RunTrace` domain struct
      (`src/domain/events.rs`); `trace_origin()`
      (`process_strand_helpers.rs`) builds it from the delivery or the
      strand file's `event-*` front-matter; the session is merged in
      after execution (`resolved.session_id`, or the session the error
      carried per plan 071).
- [x] Trailer tests — `tests/git_versioning.rs`: 9 new tests — the
      subject is unchanged; the trailer block is present and parseable
      by `git interpret-trailers` (and is the message's last
      paragraph); `None` fields are absent; one test per field so a
      dropped field fails one test; empty trace appends no block; the
      tie-off body stays verbatim in the commit body.
- [x] The service-log line grammar gains the trace fields on the
      run-lifecycle lines — `session=`, `event=`, `source=`,
      `original-strand=` in `render_loom_event_line`
      (`src/adapters/service_log.rs`); the single-line invariant holds
      (no spaces inside values, `None` omitted). The run-lifecycle
      variants (`KnotProcessing`, `KnotCompleted`, `KnotFailed`) carry
      `trace: Option<RunTrace>` (serde-defaulted); `StrandProcessed`
      is strand-level, not run-level, and was left unchanged (see
      Deviations).
- [x] Extended `tests/consolidated_log.rs` for the new grammar fields
      — `run_lifecycle_lines_carry_trace_fields` (an `event-*` strand
      with front-matter: the processing/completed lines carry
      `event=`/`source=`/`original-strand=`, `session=` absent with
      the session-less mock, single-line invariant) and
      `plain_strand_lifecycle_lines_carry_no_trace_keys` (a plain
      strand renders no trace keys at all). Plus 6 renderer unit tests
      in `service_log.rs` (full trace per variant, partial-trace
      omission, empty-trace no-op, single-line greppability).
- [x] Verification (2026-10-23) — clippy introduces zero new warnings
      vs the Phase 2 baseline (the one the `handle_success` signature
      change introduced is allowed per repo convention); the trailer
      tests green (24 passed in `tests/git_versioning`),
      `cargo test --test consolidated_log` green (6 passed),
      `service_log` unit tests green (47 passed); full `cargo test`
      suite green (33 suites, 0 failures). Commit sha `bbb1808`
      recorded as this phase's external-verdict request. The ON-COMMIT
      suite is not required by this item.

## Deviations
- "Run-lifecycle lines" is read as the knot-run bracket —
  `KnotProcessing` / `KnotCompleted` / `KnotFailed`. `StrandProcessed`
  is strand-level (it brackets the strand, not the run, and for a
  delivered emission the run's identity is already the synthetic
  `event:<producer>/<event-id>` strand), so it was left without the
  trace field.
- The commit signature carries the four trace fields as one
  `RunTrace` struct rather than four `Option` parameters — same
  contract, one type, keeps the port at six parameters.

## Discoveries
- `extract_event_metadata` only reads metadata from files named
  `event-*` (legacy dispatched-file convention); the file-triggered
  trace therefore appears only for such strands — plain strands render
  no trace keys, which the new plain-strand consolidated-log test pins.
- The `source=` value for a file-triggered strand comes from the
  front-matter `target-knot` key (the legacy dispatched-file field
  name); the delivery path uses `producer_knot` directly.

## Notes
- The trace fields are already carried end-to-end after Phase 2
  (delivery → pipeline); this phase only makes them durable (the
  commit) and greppable (the service log) — no new plumbing upstream of
  the commit path.
