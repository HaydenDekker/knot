# Plan: Remove `input-bundling` from Prompt Template

## Problem

The `input-bundling` property on `PromptTemplate` is required in knot YAML frontmatter but has no runtime effect. It was envisioned as controlling how input files get bundled into the agent prompt (e.g. `full-file`, `diff`, `chunked`), but only `full-file` ever shipped and it is always the behaviour regardless of the value. The field is parsed, stored, validated, and then ignored — dead code in the domain, application, tests, docs, and skills.

## Target

`PromptTemplate` has only the `instructions` field. The `input-bundling` property is removed from:

- **Domain** — `PromptTemplate` struct, constructor, serialization
- **Parsing** — `RawPromptTemplate`, `KnotFile::parse()` no longer reads it
- **All tests** — unit tests, integration test fixtures, helpers
- **Docs** — user docs, design docs, API reference
- **Skills** — `knot-create`, `knot-design`
- **Rig/demo knot files** — remove the property from knot definitions

Knot files without `input-bundling` should parse cleanly. Knot files that still contain `input-bundling` should parse successfully with a parse warning (unknown property), maintaining the existing non-identified property detection.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::value_objects::tests::prompt_template_fields` | `PromptTemplate` construction, validation, empty field errors | ✅ Green — asserts `input_bundling` and `instructions` |
| `domain::value_objects::tests::prompt_template_serialization` | JSON round-trip of `PromptTemplate` | ✅ Green — serialises `input_bundling` |
| `domain::knot_file::tests::parses_valid_knot_file` | Full knot file parse including frontmatter | ✅ Green — asserts `input_bundling == "full-file"` |
| `domain::knot_file::tests::parses_valid_knot_file_with_extra_fields` | Extra YAML fields are tolerated | ✅ Green — asserts `input_bundling` |
| `domain::knot_file::tests::*` (many) | Missing fields, whitespace, round-trip, git-versioned | ✅ Green — all include `input-bundling` in fixtures |
| `domain::entities::tests::*` | `Knot` entity construction | ✅ Green — all construct `PromptTemplate` with `input_bundling` |
| `domain::events::tests` | `KnotAdded` event construction | ✅ Green — includes `input_bundling` |
| `application::usecases::tests::*` | ProcessStrand, ManageKnot, ConfigEventHandler | ✅ Green — all test fixtures include `input_bundling` |
| `adapters::outbound::loom_repository::tests::*` | Knot file scanning, prompt template parsing | ✅ Green — asserts `input_bundling == "full-file"` |
| `adapters::outbound::event_source::tests::*` | Knot file modification events | ✅ Green — test fixtures include `input-bundling` |
| `tests/helpers.rs` | `make_knot_content()` helper | ✅ Green — generates `input-bundling: "full-file"` |
| `tests/git_versioning.rs` | Git versioning integration | ✅ Green — knot fixtures include `input-bundling` |
| `tests/rig_cli.rs` | Rig CLI integration | ✅ Green — knot fixtures include `input-bundling` |

## Test Gaps

None — existing tests cover all the code paths that will change. The plan is to update fixtures, not add new behaviour.

## Phases

### Phase 0: Domain — Remove `input_bundling` from `PromptTemplate`

Hex layer: **Domain**

- [x] Remove `input_bundling` field from `PromptTemplate` in `src/domain/value_objects.rs`
- [x] Simplify `PromptTemplate::new()` to accept only `instructions: String`
- [x] Update domain tests: `prompt_template_fields`, `prompt_template_serialization`
- [x] Update `Knot` entity tests to construct `PromptTemplate` with single argument
- [x] Update `KnotAdded` event test fixture in `src/domain/events.rs`
- [x] Compile + test

### Phase 1: Parsing — Remove `input-bundling` from Knot File Parser

Hex layer: **Domain**

- [x] Remove `input_bundling` from `RawPromptTemplate` in `src/domain/knot_file.rs`
- [x] Remove `input_bundling` extraction and validation from `KnotFile::parse()`
- [x] Update all knot file parsing tests (fixtures no longer need `input-bundling`, but existing fixtures with `input-bundling` should still parse — it becomes an unknown property triggering a parse warning)
- [x] Compile + test

### Phase 2: Application — Update Use Case Tests

Hex layer: **Application**

- [x] Update all `PromptTemplate` construction in `src/application/usecases.rs` tests to single-argument form
- [x] Compile + test

### Phase 3: Outbound Adapters — Update Repository and Event Source Tests

Hex layer: **Outbound Adapters**

- [x] Remove `input_bundling` assertion from `loom_repository.rs` tests
- [x] Remove `input-bundling` from test fixtures in `event_source.rs` (or leave as unknown-property warnings — either is fine, but removing is cleaner)
- [x] Compile + test

### Phase 4: Integration Tests and Helpers

- [x] Remove `input-bundling` from `tests/helpers.rs` `make_knot_content()`
- [x] Remove `input-bundling` from `tests/git_versioning.rs` fixtures
- [x] Remove `input-bundling` from `tests/rig_cli.rs` fixtures
- [x] Full integration test run

### Phase 5: Docs, Skills, and Rig Files

- [ ] Update `docs/configuration/knots.md` — remove `input-bundling` from table and examples
- [ ] Update `docs/design-guide.md` — remove `input-bundling` from knot template
- [ ] Update `docs/api-reference.md` — remove `input_bundling` from JSON example
- [ ] Update `docs/workflows/file-generation-workflow.md` — remove from examples
- [ ] Update `docs/workflows/review-workflow.md` — remove from examples
- [ ] Update `.agents/skills/knot-create/SKILL.md` — remove from schema, examples, and table
- [ ] Update `.agents/skills/knot-design/SKILL.md` — remove from examples
- [ ] Update `demo/knot-test/review-knot.md` — remove property
- [ ] Update `rig/new-loom/review-knot.md` — remove property
- [ ] Update `rig/workflow-loom/review-knot.md` — remove property
- [ ] No compile needed — documentation only

### Phase 6: Full Test Run and Verification

- [ ] `cargo test` — all tests pass
- [ ] `cargo clippy` — clean
- [ ] Verify `rig/state.json` output no longer contains `input_bundling` in knot definitions

## Notes

- This is a **breaking change** for existing knot files — knot files containing `input-bundling` will get an unknown-property parse warning but will still function. The field is required in the old format, so removing it means existing knot YAML becomes slightly invalid (one unknown field) rather than breaking.
- The PRD (`prd-ai-driven-file-generation.md`) describes `input-bundling` conceptually as "the rules for how the input file(s) should be bundled into the prompt." The PRD doesn't need updating — it describes the intent, not the implementation. The gap between intent and implementation is that bundling strategy is now implicitly `full-file` with no configuration surface.
