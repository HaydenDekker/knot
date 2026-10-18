# Phase 3: The dedup norm on the per-rig instruction surface

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 2 is green and committed.**

## Checklist
- [x] Move the dedup norm ("if a queued event already has the same facts —
      compare frontmatter keys such as plan/phase — do not queue a second
      signal") into the rig-wording home: the Event Content Guide bullet
      ("**Deduplication**") in the content template
      (`.agents/skills/knot-template/knot-tie-off-content.md` — the source of
      the binary's hard-coded default content section via `include_str!`, so
      no code change for the default path).
- [x] The injected `## Queued Events` section references the rig's wording:
      the section points at "the event-content norm in your tie-off
      instructions", and the norm names the *Queued Events* section (wording,
      not the literal `## ` heading — see Deviations). Both appear in the
      same built prompt (conformance assertions added).
- [x] Conformance: prompt-template + instruction-surface conformance tests
      green — new `default_content_instruction_carries_dedup_norm`
      (instruction surface) + the prompt-level coexistence assertion in
      `queued_events_from_wired_queue_use_queue_timestamps` (prompt template).
- [x] Verification — `cargo clippy --all-targets` clean; `cargo test --lib`
      green (1143 passed — including the instruction-surface and prompt
      panes); commit sha `⟨TBD⟩` recorded as this phase's external-verdict
      request. The ON-COMMIT suite is not required by this item.

## Deviations
- **Norm wording references the section by name, not by literal heading.**
  The first draft wrote "see the `## Queued Events` section" — the content
  section is injected into *every* producer prompt (not just queue-non-empty
  ones), so the literal heading string leaked into prompts where the section
  is omitted and broke the omit-when-empty assertions. The norm now says "see
  the *Queued Events* section of your prompt, when present".
- **Pre-existing process_strand prompt assertions updated here, not in
  Phase 2.** Three process_strand tests
  (`prompt_contains_pending_events_when_files_exist`,
  `producer_re_enters_prompt_contains_pending_event`,
  `producer_with_multiple_event_types_sees_relevant_pending_events`)
  asserted the old `## Pending Events` heading + description carry-over. They
  are prompt-content tests — this phase's pane — and were re-asserted to the
  new contract (section present, description NOT carried) in this commit.

## Discoveries
- The content template is already in production sync state (the 096-series
  deploy had landed); the only prod diff after this change is the new
  bullet. Deployed + diff-verified per AGENTS.md
  (`~/.agents/skills-library/knot-template/`).

## Notes
- The norm is guidance; the rig's `knot-manage` review remains the
  duplicate-signal detector (Target 3 is the queue-busy observation).
