# Phase 5: Acceptance through the adapters + rig docs

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [ ] Acceptance tests (full flow through the adapter layer): mock CLI script (`KNOT_TEST_CLI_PATH` pattern) records its argv; resolve a `model-ref` profile against a real `models.yml` containing an alias default, with the profile overriding it; run `PiStdioAgentRunner` **and** `PiJsonAgentRunner`; assert the spawned argv contains `--thinking <effective-level>` in both
- [ ] Update the auto-created `models.yml` template comment (`src/server.rs`) to document the new key
- [ ] Update the `knot-create` skill (profile frontmatter table + models.yml section + example)
- [ ] Update the `knot-inspect` skill (profile listing)
- [ ] Update the `knot-init` skill (models.yml seeding note, where applicable)
- [ ] Add the `knot-update` changelog entry (field additions; effective-level resolution; `off`-vs-omission asymmetry)
- [ ] Full suite — no regressions in existing arg-shape or state-shape tests
- [ ] Publish updated skills globally (`~/.agents/skills/`) and verify with diff

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
