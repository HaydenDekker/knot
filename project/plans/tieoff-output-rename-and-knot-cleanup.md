# Plan: Tie-Off Output Rename and Knot File Cleanup

## Problem

Two issues remain from the static-output-paths refactor:

1. **Terminology mismatch** — The output directory is named `output` and tie-off files are named `{strand-name}.output`, but the domain concept is "tie-off". The directory should be `tie-offs` and each knot's artifact should be `{knot-name}-tie-off.md`. This is a single append-mode file per knot, not one file per strand.

2. **Dead `tie-off-dir` coupling** — `RawFrontmatter` in `knot_file.rs` still accepts `tie-off-dir` from YAML frontmatter (accepted but ignored). This is legacy from before static paths. Users can add arbitrary YAML fields to knot files with no indication they are unused, which is confusing.

## Target

1. Directory: `rig/output/` → `rig/tie-offs/`
2. Filename: `{strand-name}.output` → `{knot-name}-tie-off.md` (one file per knot)
3. `RawFrontmatter.tie_off_dir` removed from `knot_file.rs`
4. Unknown YAML properties in knot frontmatter emit a `.loom-log` warning so the user is informed
5. `project/domain-glossary.md` updated to reflect new paths

## Implementation Status: ✅ Complete (2026-06-12)

## Existing Tests

| Test File | Test | What it covers | Status |
|-----------|------|---------------|--------|
| `knot_file.rs` | `valid_knot_file_parse` | Knot YAML parsing with all required fields | ✅ Green |
| `knot_file.rs` | `tieoff_dir_in_yaml_is_accepted_but_ignored` | `tie-off-dir` in YAML is parsed but not stored | ✅ Green |
| `knot_file.rs` | `derive_tieoff_path_builds_correct_path` | `rig/output/{loom-id}/{knot-name}/` | ✅ Green |
| `knot_file.rs` | `derive_loom_log_path_builds_correct_path` | `rig/output/{loom-id}/.loom-log` | ✅ Green |
| `knot_file.rs` | `missing_strand_dir_returns_error` | Knot with `tie-off-dir` but no `strand-dir` | ✅ Green |
| `loom_repository.rs` | `scan_parses_per_knot_source_and_tieoff_dirs` | Scanner reads knots with `tie-off-dir` in YAML | ✅ Green |
| `loom_repository.rs` | `scan_skips_non_loom_directories` | `output/` directory is skipped during scan | ✅ Green |
| `loom_repository.rs` | `scan_requires_strand_dir` | Knot without `strand-dir` is skipped | ✅ Green |
| `usecases.rs` | `compute_tie_off_path` | Filename derivation: `{strand}.output` | ✅ Green |
| `tieoff_sink.rs` | `tieoff_filename_derived_from_strand` | `FileSystemTieOffSink.derive_tieoff_filename` (dead code) | ✅ Green |
| `tie_off.rs` (integration) | `full_tie_off_history` | E2E: path `output/{loom}/{knot}/{strand}.output` | ✅ Green |
| `tie_off.rs` (integration) | `tie_off_sections_readable` | E2E: path `output/{loom}/{knot}/{strand}.output` | ✅ Green |
| `loom.rs` (integration) | `post_loom_without_tieoff_dir_succeeds` | POST /looms without tie-off-dir succeeds | ✅ Green |

## Test Gaps

- No test for non-identified YAML property detection (new feature)
- No test verifying `.loom-log` receives warning entries (new feature)
- Integration tests hard-code `output/` path — must update assertions

## Phases

### Phase 0: Domain — add non-identified property detection to KnotFile parser

Hex layer: Domain (`knot_file.rs`)

- [x] Add `#[serde(flatten)]` field `extra: HashMap<String, serde_yaml::Value>` to `RawFrontmatter` to capture any YAML keys not matched by named fields
- [x] Remove `tie_off_dir` field from `RawFrontmatter` entirely (it was already ignored)
- [x] Change `parse()` return type from `Result<KnotFile, KnotFileError>` to return `(KnotFile, Vec<String>)` where the `Vec<String>` contains warning messages for each unknown property
  - Warning format: `"unknown property '{key}' in knot frontmatter (not used)"`
- [x] Update `parse()` tests: `valid_knot_file_parse`, `tieoff_dir_in_yaml_is_accepted_but_ignored` (becomes `unknown_property_emits_warning`), `missing_strand_dir_returns_error` (remove `tie-off-dir` from fixture)
- [x] New test: `parse_detects_unknown_properties` — knot YAML with `foo: bar` produces `KnotFile` + warning
- [x] New test: `parse_no_warnings_for_valid_knot` — standard knot YAML produces `KnotFile` + empty warnings

### Phase 1: Domain — rename output directory and update path derivation

Hex layer: Domain (`knot_file.rs`)

- [x] Rename `derive_tieoff_path`: `"output"` → `"tie-offs"` in `rig.join("tie-offs").join(loom_id).join(knot_name)`
- [x] Rename `derive_loom_log_path`: `"output"` → `"tie-offs"` in `rig.join("tie-offs").join(loom_id).join(".loom-log")`
- [x] Update docstrings on both functions
- [x] Update `derive_tieoff_path_builds_correct_path` test: expected path `tie-offs/my-loom/review-knot`
- [x] Update `derive_loom_log_path_builds_correct_path` test: expected path `tie-offs/my-loom/.loom-log`
- [x] Verify compile: `cargo build`
- [x] Update integration test hardcoded `output/` → `tie-offs/` paths (14 test files, ~48 references)

### Phase 2: Application — rename tie-off filename from strand-based to knot-based

Hex layer: Application (`usecases.rs`)

- [x] In `ProcessStrand::compute_tie_off_path`, replace:
  - `format!("{}.output", strand_filename)` → `{knot-id}-tie-off.md`
  - The path is now `{knot-name}-tie-off.md` (one file per knot, not per strand)
- [x] Update docstring on `compute_tie_off_path`
- [x] Update the `unwrap_or_else(|| "output".to_string())` fallback to `"tie-off.md"`
- [x] Updated 13 integration test files to use `{knot-id}-tie-off.md` path format (20 assertions across 10 test files)

### Phase 3: Outbound — propagate warnings from parser through scanner to loom-log

Hex layers: Outbound adapter (`loom_repository.rs`) → Application use case (`usecases.rs`)

- [x] `loom_repository.rs` — `scan_knot_files`: capture warnings from `knot_file_parser::parse()`. Change return type to propagate warnings. Since `scan_knot_files` returns `Result<Vec<Knot>, PortError>`, add a new return variant or use a wrapper struct `ScanResult { knots: Vec<Knot>, warnings: Vec<String> }`
- [x] `loom_repository.rs` — `scan()`: propagate warnings from `scan_knot_files` into loom-level results. Since `scan` returns `Vec<Loom>`, the warnings must propagate at a different level.
  - **Approach**: The `Loom` entity doesn't carry warnings. Instead, the scanner returns `Result<(Vec<Loom>, Vec<String>), PortError>` or the warnings are emitted at parse time via a callback.
  - **Simpler approach**: `knot_file_parser::parse()` writes warnings to the loom-log directly? No — domain layer cannot do IO.
  - **Correct approach**: `parse()` returns warnings. `scan_knot_files()` collects them. `scan()` collects them per-loom. The caller (use case) writes them to loom-log. This means `LoomRepository::scan()` needs to return warnings too.
- [x] **Port change**: `LoomRepository::scan()` signature changes from `fn scan(&self, rig: &Path) -> Result<Vec<Loom>, PortError>` to `fn scan(&self, rig: &Path) -> Result<(Vec<Loom>, Vec<String>), PortError>` where `Vec<String>` is all warnings across all looms
- [x] **Implementation**: Update `FileSystemLoomRepository::scan()` to collect warnings from `scan_knot_files` and return them
- [x] **Use case**: `DiscoverLooms::execute()` and `RegisterLoom` receive warnings and write them as `LoomEvent` entries. Need a `LoomEvent::Warning` variant.
- [x] **LoomEvent**: Add `LoomEvent::KnotParseWarning { loom_id, knot_file_name, message, timestamp }` variant (or simpler: `LoomEvent::Warning` with free-text message).
- [x] **Mock**: Update `MockLoomRepository` in `ports.rs` tests to return empty warnings tuple
- [x] Update `scan_skips_non_loom_directories` test: expected scan directory is now `tie-offs/` not `output/`
- [x] Update `scan_parses_per_knot_source_and_tieoff_dirs` test: remove `tie-off-dir` from knot fixtures (field is no longer accepted)
- [x] Update `scan_requires_strand_dir` test: fixtures no longer reference `tie-off-dir`
- [x] New test: `scan_returns_warnings_for_unknown_properties` — scanner detects extra YAML fields and returns warning strings
- [x] Verify compile: `cargo build`
- [x] Updated all callers: `DiscoverLooms::execute`, `register_single`, `ConfigEventHandler` (handle_loom_added, register_loom), mock repos in tests/ files, `ports.rs` tests, `usecases.rs` tests
- [x] Updated `KnotParseWarning` serialization test in `events.rs`

### Phase 4: Integration tests — update path expectations

Hex layer: Integration tests (`tests/tie_off.rs`, `tests/loom_api.rs`)

- [x] `tests/tie_off.rs` — `full_tie_off_history`: update `tie_off_path` from `output/history-loom/review-knot/lifecycle-strand.md.output` to `tie-offs/history-loom/review-knot/review-knot-tie-off.md`
- [x] `tests/tie_off.rs` — `tie_off_sections_readable`: update `tie_off_path` from `output/sections-loom/review-knot/sections-strand.md.output` to `tie-offs/sections-loom/review-knot/review-knot-tie-off.md`
- [x] Verify: `cargo test` (full suite) — all 120+ tests pass

Note: These paths were already updated in Phase 2 when the sub-agent updated all 13 test files.

### Phase 5: Documentation — update domain glossary

Hex layer: Documentation (`project/domain-glossary.md`)

- [x] Update all references from `rig/output/` to `rig/tie-offs/`
- [x] Update tie-off directory description: path is `rig/tie-offs/{loom-id}/{knot-name}/`
- [x] Update tie-off file description: filename is `{knot-name}-tie-off.md` (one per knot)
- [x] Update term relationships diagram
- [x] Update loom-log path: `rig/tie-offs/{loom-id}/.loom-log`
- [x] Remove references to `output.md` as the tie-off filename
- [x] Update AGENTS.md if it references output paths — AGENTS.md only references the glossary file, no direct output paths

### Phase 6: Cleanup — remove dead code

Hex layer: Outbound adapter (`tieoff_sink.rs`)

- [x] `FileSystemTieOffSink::derive_tieoff_filename` and `resolve_path` are dead code (not called by `ProcessStrand` which computes its own path). Remove or mark as `#[allow(dead_code)]` with a comment explaining they are unused legacy.
- [x] Clean up related tests in `tieoff_sink.rs`: `tieoff_filename_derived_from_strand`, `tieoff_filename_no_extension`, `tieoff_filename_complex_extension`, `tieoff_resolve_path`
- [x] Verify compile: `cargo build` — zero warnings
- [x] Verify full test suite: `cargo test` — 257+ tests pass

Note: Added `#[allow(dead_code)]` on `tie_off_dir` field to suppress unused field warning while preserving struct API.

### Phase 7: Skills — update agent skill documentation

Hex layer: Skills (`skills/knot-*/SKILL.md`) — local skills are source of truth, copied to global after.

- [x] `skills/knots-and-looms/SKILL.md` — remove all `tie-off-dir` / `tie_off_dir` references (field is no longer accepted in knot YAML):
  - Lines 62–63: remove `tie_off_dir` from required/optional fields description
  - Line 74: remove `"tie_off_dir": "output/prds"` from JSON example
  - Line 85: remove `output to \`output/prds\`` from example sentence
  - Line 144: remove `tie-off-dir: "output"` from YAML example
  - Line 166: remove `tie-off-dir` column entry from knot fields table
  - Lines 172, 176: remove `tie-off-dir` / `tie_off_dir` from path resolution notes
  - Line 182: remove `output/` from directory tree example
  - Lines 204–205: remove `tie-off-dir` from knot YAML description
  - Line 253: remove `tie_off_dir` from schema example
  - Lines 270, 283: remove `"tie_off_dir"` from JSON request/response examples
  - Line 320: update prose about "writing output to one tie-off directory" — tie-off dir is now static `rig/tie-offs/{loom-id}/{knot-name}/`
  - Line 337: remove `tie_off_dir` from curl example
  - Add note: tie-off paths are static — `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`
- [x] `skills/knot-inspect/SKILL.md` — update path examples to new naming:
  - Line 60: update table example `output/prds` → `tie-offs/`
  - Lines 150, 161: update `"tie_off_dir"` in JSON examples (field removed or path updated)
  - Line 209: update `"tie_off_path": "output/input.md.output"` → `"tie_off_path": "tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md"`
- [x] `skills/knot-init/SKILL.md` — update API response example:
  - Line 120: update `"tie_off_dir": "output/docs"` to match new schema
- [x] Verify all three skills are internally consistent (no orphaned `output/` or `tie-off-dir` references)
- [x] Copy updated `skills/knot-*/SKILL.md` to global `~/.agents/skills/knot-*/SKILL.md`

## Notes

- The `FileSystemTieOffSink::derive_tieoff_filename` method exists but is **not called** by the processing pipeline — `ProcessStrand::compute_tie_off_path` does its own derivation. This dead code is in Phase 6.
- The `LoomRepository` port change (`scan` return type) touches the trait and all implementations. This is a Phase 3 concern.
- The warning propagation path is: `knot_file::parse()` → `scan_knot_files()` → `scan()` → `DiscoverLooms::execute()` → `LoomLogPort::append(LoomEvent::Warning)`.
- `tie-off-dir` removal from `RawFrontmatter` is Phase 0 (domain), its test fixtures in `loom_repository.rs` are Phase 3 (adapter tests).
- The `output/` directory rename in `derive_tieoff_path` and `derive_loom_log_path` means the loom-log also moves. This is purely cosmetic — the loom-log logic doesn't change.
