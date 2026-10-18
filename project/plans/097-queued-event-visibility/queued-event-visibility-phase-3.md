# Phase 3: Dedup norm on the per-rig instruction surface

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 2 is green and committed.**

## Checklist
- [ ] Move the dedup norm ("if a queued event already carries the same facts —
      compare the frontmatter keys such as plan/phase — emit `event: None`
      instead of queueing a second signal") into the `knot-template` instruction
      content: the shipped content template
      `.agents/skills/knot-template/knot-tie-off-content.md` (the source of the
      binary's hard-coded default content section via `include_str!`,
      `src/domain/events.rs`), and the prompt template that references the
      rig's wording.
- [ ] Keep the hard-coded core (the tie-off contract, event format, rules —
      `DEFAULT_TIE_OFF_INSTRUCTION`) untouched; only the content section moves.
- [ ] The injected prompt references the rig's wording (per-rig override via the
      `tie-off-content-instruction-file` link in `rig/.workspace-agent-config.yaml`,
      pulled fresh each session — `PromptInstructionSource`).
- [ ] Add/adjust conformance tests: prompt-template + instruction-surface
      tests — the content template carries the norm; the prompt references it;
      a per-rig override file replaces the default content section (the
      `FixedInstructions`-style double in the context-provider tests).
- [ ] Deploy the updated skill (copy `.agents/skills/knot-template/` to
      `~/.agents/skills-library/knot-template/`, including the companion
      `knot-tie-off.md` / `knot-tie-off-content.md`) and verify by diff per
      AGENTS.md.
- [ ] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      events` + `cargo test --lib context_providers` green (prompt-template +
      instruction-surface conformance — the phase's pane); commit sha `⟨TBD⟩`
      recorded as this phase's external-verdict request. The ON-COMMIT suite is
      not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Note for the `knot-update` skill and the release notes: producer prompts
  change when the queue is non-empty (additive — existing rigs can ignore the
  section).
