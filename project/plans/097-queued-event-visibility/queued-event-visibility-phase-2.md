# Phase 2: Prompt injection of the Queued Events section

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 1 is green and committed.**

## Checklist
- [ ] Rework `AgentEventsContextProvider` (`src/application/usecases/context_providers.rs`):
      build the pending section from the Phase 1 queue-backed index (per-type
      count + newest-N `{path, queued_at}`) instead of the per-file
      `description`/`timestamp` frontmatter scan; keep the frontmatter-only
      `target-knot` producer filter (or the queue-entry producer field, per the
      Phase 1 decision).
- [ ] Reword the section heading/body: `## Queued Events` — "queued, awaiting
      consumer invocation" framing; drop the typo'd 060 wording. Keep
      omit-when-empty (no section when the queue holds nothing of the knot's
      emittable types).
- [ ] Apply the cap through to the prompt (a backlog of N must not become N
      prompt entries — show count + capped newest-N).
- [ ] Per the Phase 0 fallback decision: keep `scan_dispatch_directory` as the
      queue-less fallback (capped) or remove it; record in Deviations.
- [ ] Update the existing context-provider prompt-content tests to the new
      source — old directory-scan assertions → queue-backed assertions; add the
      prompt-level cap assertion (50 queued → 10 listed); keep/adjust the
      omit-when-empty test.
- [ ] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      context_providers` green (the context-provider prompt-content suite — the
      phase's pane) + `cargo test --test pipeline` green (full-pipeline context
      injection still holds); commit sha `⟨TBD⟩` recorded as this phase's
      external-verdict request. The ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Emission and enforcement flows untouched — this is a prompt-content change
  only.
