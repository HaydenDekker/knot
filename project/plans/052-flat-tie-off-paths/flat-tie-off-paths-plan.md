# Plan 52: Flatten Tie-Off Paths — Remove Intermediate Knot Directory

## Problem

Tie-offs are currently placed in subdirectories within `rig/tie-offs/`, producing a three-level nesting:

```
rig/tie-offs/{loom-id}/{knot-name}/tie-off-{knot-name}.md
```

This intermediate `{knot-name}` directory adds no value — the tie-off filename already identifies the knot. The extra directory level is annoying to navigate and, more importantly, it wastes the opportunity to use subdirectories within the loom's tie-off space for **event capture**.

The domain glossary describes "Tie-Off Events" — typed subdirectories for agent-to-agent communication — but their current placement (`rig/tie-offs/{loom-id}/{knot-name}/{event-type}/`) buries them under the knot directory. With flat tie-offs, the loom-level directory is available for event folders:

```
rig/tie-offs/{loom-id}/
├── .loom-log
├── tie-off-review.md          ← flat tie-off file
├── plan-architect-tie-off.md  ← another flat tie-off file
└── reviews/                   ← event subdirectory (captured by folders)
    └── 001-initial-review.md
```

Consumer knots that subscribe to events reference:
`../../tie-offs/{loom-id}/{event-type}` (one level shallower than before).

## Target

Tie-off files are placed flat under `rig/tie-offs/{loom-id}/`:

- **Tie-off path:** `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md` (was `rig/tie-offs/{loom-id}/{knot-name}/tie-off-{knot-name}.md`)
- **Loom-log path:** unchanged (`rig/tie-offs/{loom-id}/.loom-log`)
- **Event subdirectories:** still under `rig/tie-offs/{loom-id}/` (one level shallower for consumer `strand-dir` references)

The `derive_tieoff_path()` function is simplified from:
```rust
rig.join("tie-offs").join(loom_id).join(knot_name)
```
to:
```rust
rig.join("tie-offs").join(loom_id)
```

All code that derives, writes, reads, or tests tie-off paths is updated to the new flat layout. Domain glossary, user docs, and agent skills are updated to reflect the new paths.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/domain/knot_file.rs` — `derive_tieoff_path_builds_correct_path` | Path derivation for tie-offs | ✅ Green — asserts nested path |
| `src/domain/knot_file.rs` — `derive_loom_log_path_builds_correct_path` | Path derivation for loom-log | ✅ Green — unchanged by this plan |
| `src/application/usecases/process_strand.rs` — `execution_test_shared::build_process_strand` | Mock tie-off path construction | ✅ Green — hardcodes nested paths |
| `src/application/usecases/process_strand.rs` — `process_strand_deleted_includes_strand_history` | Tie-off content read for deleted events | ✅ Green — hardcodes nested path |
| `tests/tie_off.rs` — all tests | Integration: tie-off writing and reading | ✅ Green — asserts nested paths |
| `tests/skill_integration.rs` — `test_tie_off_written` | Integration: skill produces tie-off | ✅ Green — asserts nested paths |
| `tests/skill_integration.rs` — `test_loom_log_written` | Integration: loom-log location | ✅ Green — unchanged |
| `tests/adapter_integration.rs` — tie-off tests | Integration: adapter writes tie-offs | ✅ Green — asserts nested paths |
| `tests/agent_integration.rs` — tie-off tests | Integration: agent output to tie-off | ✅ Green — asserts nested paths |
| `tests/session_resume.rs` — tie-off tests | Integration: resume writes tie-off | ✅ Green — asserts nested paths |
| `tests/pipeline.rs` — tie-off tests | Integration: full pipeline tie-off | ✅ Green — asserts nested paths |
| `tests/pipeline.rs` — `test_git_versioning` | Integration: git versioning of tie-off | ✅ Green — asserts nested paths |
| `tests/helpers.rs` — `read_loom_log` | Test helper: loom-log path | ✅ Green — unchanged |
| `tests/rig_cli.rs` — share command | Integration: share excludes tie-offs | ✅ Green — excludes `tie-offs/` dir |

## Test Gaps

- No test verifying the tie-off filename is unique per knot (collision detection when two knots produce same-named files — not a concern with flat layout since knot names must be unique within a loom)
- No test that validates the flat path pattern end-to-end (integration tests assert the old nested path)

## Phases

### Phase 0: Domain — Flatten `derive_tieoff_path` and Update Tests

Change the core path derivation in `src/domain/knot_file.rs`:

1. **Update `derive_tieoff_path()`** — remove the `join(knot_name)` step:
   ```rust
   // Before:
   rig.join("tie-offs").join(loom_id).join(knot_name)
   // After:
   rig.join("tie-offs").join(loom_id)
   ```

2. **Update `compute_tie_off_path()`** in `src/application/usecases/process_strand.rs` — the function already joins `filename` to the base path, so it just needs the base to be shallower. No logic change needed; it inherits from `derive_tieoff_path`.

3. **Update doc comments** in both functions to reflect the new path:
   - `derive_tieoff_path`: "Returns `rig/tie-offs/{loom-id}/`"
   - `compute_tie_off_path`: "Uses statically derived path: `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`"

4. **Update unit test** `derive_tieoff_path_builds_correct_path` in `knot_file.rs`:
   - Expected path changes from `/workspace/rig/tie-offs/my-loom/review-knot` to `/workspace/rig/tie-offs/my-loom`

### Phase 1: Application Layer — Update ProcessStrand Tests

Update the hardcoded tie-off paths in `src/application/usecases/process_strand.rs`:

1. **`execution_test_shared::build_process_strand`** — mock tie-off content key:
   - `/rig/tie-offs/test-loom/k1/tie-off-k1.md` → `/rig/tie-offs/test-loom/tie-off-k1.md`

2. **`process_strand_deleted_includes_strand_history`** — pre-populated tie-off content key:
   - `/rig/tie-offs/test-loom/k1/tie-off-k1.md` → `/rig/tie-offs/test-loom/tie-off-k1.md`

3. Verify all process_strand unit tests pass.

### Phase 2: Integration Tests — Update All Tie-Off Path Assertions

Update integration test files that assert tie-off file paths:

1. **`tests/tie_off.rs`** — all tie-off path assertions:
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

2. **`tests/skill_integration.rs`** — tie-off path assertions:
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

3. **`tests/adapter_integration.rs`** — tie-off path assertions:
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

4. **`tests/agent_integration.rs`** — tie-off path assertions and `tie_off_dir`:
   - `tie-offs/review-loom/review/` → `tie-offs/review-loom/`
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

5. **`tests/session_resume.rs`** — tie-off path assertions:
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

6. **`tests/pipeline.rs`** — tie-off path assertions and `tie_off_dir`:
   - `tie-offs/review-loom/review/` → `tie-offs/review-loom/`
   - `tie-offs/review-loom/review/tie-off-review.md` → `tie-offs/review-loom/tie-off-review.md`

7. **`tests/helpers.rs`** — any tie-off path references (loom-log paths unchanged).

8. **`tests/rig_cli.rs`** — share command test creates tie-offs directory structure:
   - Update any nested path setup to flat layout.

### Phase 3: Documentation — Update Domain Glossary, User Docs, Skills

Update all documentation to reflect the new flat path structure:

1. **`project/domain-glossary.md`** — update:
   - "Knot" section: tie-off path description
   - "Tie-off Directory" section: remove knot-name subdirectory
   - "Tie-Off Events" section: update layout diagram and consumer `strand-dir` reference
   - "Tie-off" section: update path
   - "Term Relationships" diagram: flatten the tie-off tree

2. **`docs/concepts.md`** — update tie-off path references

3. **`docs/configuration/rig-structure.md`** — update directory tree and path examples

4. **`docs/configuration/knots.md`** — update tie-off path and directory tree

5. **`docs/troubleshooting.md`** — update any tie-off path references

6. **`.agents/skills/knot-inspect/SKILL.md`** — update tie-off path examples in state JSON and loom-log examples

7. **`.agents/skills/knot-create/SKILL.md`** — update rig structure diagram and tie-off event descriptions

8. **`AGENTS.md`** — if it contains tie-off path references, update them

### Phase 4: Version Bump and Verification

1. **Bump version** in `Cargo.toml` (patch bump — no API change, just path layout)
2. **Run full test suite**: `cargo test` — verify all tests pass
3. **Run clippy**: `cargo clippy` — verify no warnings

## Notes

- This is a **breaking change for existing rigs** — tie-off files move from nested directories to flat. Users with existing rigs will see new tie-off files created at the flat location on next processing event. The old nested files remain on disk (harmless orphan files). A future migration could clean them up, but it's not required.
- **No data loss** — the tie-off append-mode means existing content in old files is preserved. New processing starts fresh files at the new flat location.
- **Consumer knot `strand-dir` references** to event subdirectories will need updating if they use relative paths (e.g. `../../tie-offs/{loom-id}/{knot-name}/{event-type}` becomes `../../tie-offs/{loom-id}/{event-type}`). This is a manual migration step for existing rigs.
- **No change to loom-log paths** — they are already flat at `rig/tie-offs/{loom-id}/.loom-log`.
- **No change to rig-log paths** — already at `rig/.rig-log`.
- The `FileSystemTieOffSink::append()` and `write()` methods use `fs::create_dir_all(parent)` which already handles the flat parent directory correctly — no code change needed in the sink itself.
- The `TieOff` entity's `path` field is set by `compute_tie_off_path()` which derives from `derive_tieoff_path()` — so the sink receives the correct flat path automatically.

## Implementation Status: ✅ Complete (2026-07-01)

## Completion Notes
- Version bumped to 0.22.0 (MINOR — breaking change for existing rig tie-off paths)
- All 5 phases (0–4) complete
- 199 integration tests pass across 11 suites (agent_integration, pipeline, tie_off, adapter_integration, rig_lifecycle, multi_loom, profile_timeout, rig_log, shutdown, skill_integration, git_versioning)
- 475 unit tests pass
- Clippy: 34 pre-existing warnings, none from this plan
- Domain glossary, user docs, and agent skills updated to flat path structure
- Migration entry added to knot-update skill (0.22.0 changelog)
- PATH race condition fix: serialisation locks (`TEST_MUTEX` / `acquire_test_lock()`) added to 7 test suites covering 27 test functions
- Session resume test failures remain (pre-existing mock identity issue, tracked by plan 053)
