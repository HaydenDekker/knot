# Phase 2: Prompt injection of the Queued Events section

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 1 is green and committed.**

## Checklist
- [x] Rework `AgentEventsContextProvider` (`src/application/usecases/context_providers.rs`):
      build the pending section from the Phase 1 queue-backed index (per-type
      count + newest-N `{path, queued_at}`) instead of the per-file
      `description`/`timestamp` frontmatter scan; keep the frontmatter-only
      `target-knot` producer filter (per the Phase 1 decision, D1).
- [x] Reword the section heading/body: `## Queued Events` — "queued, awaiting
      consumer invocation" framing; drop the typo'd 060 wording. Keep
      omit-when-empty (no section when the queue holds nothing of the knot's
      emittable types). The section now points at the event-content norm in the
      tie-off instructions (its new home — Phase 3).
- [x] Apply the cap through to the prompt (a backlog of N must not become N
      prompt entries — show count + capped newest-N; the heading reads
      `N queued (newest 10 shown)`).
- [x] Per the Phase 0 fallback decision (D2): keep `scan_dispatch_directory`
      as the queue-less fallback, capped like the queue path (it feeds the
      same `build_pending_index`, so the cap applies uniformly).
- [x] Update the existing context-provider prompt-content tests to the new
      source — old directory-scan assertions → the new `## Queued Events`
      format (count + path, no description carry-over); add the prompt-level
      cap assertion (50 queued → `50 queued (newest 10 shown)`, entry 49
      listed, entry 39 not); keep/adjust the omit-when-empty test. New
      queue-wired tests: `queued_events_from_wired_queue_use_queue_timestamps`
      (queue `queued_at` used, frontmatter ts/description do not leak),
      `queued_events_backlog_is_capped_at_newest_ten`,
      `queued_events_omitted_when_only_other_producers_queued`,
      `queued_events_scoped_to_emittable_types` (via a `FakeQueue` accessor
      double).
- [x] Verification — `cargo clippy --all-targets` clean (warning count
      unchanged from the Phase 0 baseline — the two `collapsible_if` notes in
      this file are in the pre-existing `scan_dispatch_directory` /
      `extract_frontmatter` helpers, not new code); `cargo test --lib
      context_providers` green (13 passed — the context-provider
      prompt-content suite, the phase's pane) + `cargo test --test pipeline`
      green (19 passed — full-pipeline context injection still holds); commit
      sha `d6cc16a` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
- The `## Pending Events` heading was renamed to `## Queued Events` (the
  plan's "reword the section"). Prompt-surface migration note for the
  `knot-update` skill / release notes: the section heading, wording, and
  entry shape change when the queue is non-empty (additive — rigs can ignore
  the section; the heading rename is cosmetic and only appears when the
  queue is non-empty).
- Producer filter stays a frontmatter-only `target-knot` read (Phase 1 D1);
  the `description`/`timestamp` frontmatter fields are no longer read or
  carried — the index is metadata-only.

## Discoveries
- The directory-scan fallback has no `queued_at` (paths only); it feeds the
  same index with an empty timestamp, so the section omits the `(queued: …)`
  suffix in queue-less contexts (cosmetic; the fallback is for tests /
  partial wiring only).
- No `run-scope.md` / `scripts/test-data/` — verification uses the targeted
  `cargo test` filters as the ALWAYS-equivalent.

## Notes
- Emission and enforcement flows untouched — this is a prompt-content change
  only.
