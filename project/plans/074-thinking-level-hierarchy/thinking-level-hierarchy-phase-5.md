# Phase 5: Acceptance through the adapters + rig docs

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [x] Acceptance tests (full flow through the adapter layer): mock CLI script (`KNOT_TEST_CLI_PATH` pattern) records its argv; resolve a `model-ref` profile against a real `models.yml` containing an alias default, with the profile overriding it; run `PiStdioAgentRunner` **and** `PiJsonAgentRunner`; assert the spawned argv contains `--thinking <effective-level>` in both — `tests/thinking_level.rs`: profile `analyst` (`model-ref: frontier` + `thinking-level: xhigh`) resolved against a registry with alias default `high`; both runners spawn the mock CLI; argv asserted to carry `--thinking xhigh` (profile override, not the alias default) after `--model`
- [x] Update the auto-created `models.yml` template comment (`src/server.rs`) to document the new key
- [x] Update the `knot-create` skill (profile frontmatter table + models.yml section + example) — 5.7.0, compatibility 0.36.0+
- [x] Update the `knot-inspect` skill (profile listing) — 3.6.0, compatibility 0.36.0+ (state schema example, profile-entry paragraph, list/view tables gain the effective `thinking-level`)
- [x] Update the `knot-init` skill (models.yml seeding note, where applicable) — 4.3.0, compatibility unchanged (see Notes)
- [x] Add the `knot-update` changelog entry (field additions; effective-level resolution; `off`-vs-omission asymmetry) — 1.12.0, compatibility 0.36.0+
- [x] Full suite — 862 lib + binary + all integration suites green, 0 failures (arg-shape and state-shape tests unaffected)
- [x] Publish updated skills globally (`~/.agents/skills/`) and verify with diff — all nine skills OK

## Deviations
None — implemented exactly as planned.

## Discoveries
- Phase 5 was partially drafted before this run (uncommitted): the acceptance tests (`tests/thinking_level.rs`), the `src/server.rs` template comment, and the `knot-create` content update already existed. This run verified the acceptance tests green, added the missing skill version/compatibility bumps, and completed `knot-inspect`, `knot-init`, `knot-update`, the master-plan line, and the global publish.
- The mock CLI is a two-line bash script: `printf '%s\n' "$@"` records argv (one arg per line) to `argv.txt` next to the script; `cat` echoes stdin back as the agent's stdout so both runners see a successful run. The json runner's `--mode json` flag is appended after the base args — the `--thinking` position assertion (after `--model`) holds in both.
- Registry vs profile error asymmetry (pinned in Phase 1/2, restated in the changelog entry): an invalid registry `thinking-level` degrades to a warning + empty registry (never blocks processing); an invalid profile value is a hard parse error (`InvalidThinkingLevel`).

## Notes
- Skill versions: knot-create 5.7.0, knot-inspect 3.6.0, knot-init 4.3.0, knot-update 1.12.0. Compatibility bumped to 0.36.0+ for the skills that document the new field/state key (create/inspect/update), matching the 069 precedent. knot-init keeps 0.32.0+ — its seeding note is informational and version-marked ("Knot 0.36.0+"), and pre-0.36.0 binaries ignore the unknown YAML key (no `deny_unknown_fields`), so nothing breaks either way.
- The binary MINOR bump (0.35.0 → 0.36.0) happens at plan completion, not in this phase — the changelog entry references the target version ahead of the bump, matching plan 073.
- The state.json `thinking-level` key is the **effective** level and is omitted (never `null`) when unset — the knot-inspect docs tell agents to show `default` for the absent key, mirroring the `timeout` convention.
