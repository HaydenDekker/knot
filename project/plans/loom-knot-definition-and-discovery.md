# Plan 13: Loom Naming Convention, Knot Definition Rules, and Discovery Fix

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan fixes the loom discovery and knot registration flow to match the updated domain model. It addresses user stories 1 (create a knot and confirm it is active) and 2 (watch a loom and generate tie-offs) by ensuring looms are correctly discovered by naming convention, knots are always loaded from the loom directory, and the API accepts the correct fields.

## Problem

Three bugs in the current loom/knot model prevent correct operation:

1. **Loom discovery matches any subdirectory** — `FileSystemLoomRepository::scan()` treats every subdirectory of `rig/` as a loom. This means state directories created by `LoomLogPort::open()` (e.g. `<rig>/<loom-id>/` for `.loom-log`) are discovered as looms on restart, producing phantom entries with zero knots.

2. **`POST /looms` scans `source_dir` for knot files** — The handler accepts `source_dir` and scans that path for `.md` knot files. But knot files live in the loom directory (e.g. `rig/planning-loom/planner.md`), not in the source directory. The API-registered loom gets 0 knots.

3. **`Loom.source_dir` is ambiguous** — The loom entity carries a `source_dir` that serves dual purposes: it's both the loom directory (where knot files live) and the watch target (where strands live). This conflates two distinct concepts and causes the wrong directory to be watched.

The updated domain model (per `domain-glossary.md` and `prd-ai-driven-file-generation.md`) defines:

- A loom is a directory under `rig/` whose name ends in `-loom`
- Knot `.md` files live at the first level of the loom directory
- Each knot defines a required `strand_dir` (directory to watch) and required `tie_off_dir` (output directory)
- The loom directory is static and derived from naming — not exposed via the API
- Knot `strand_dir` replaces the previous `source_dir` terminology

## Target

When this plan is done:

- `FileSystemLoomRepository::scan()` only discovers directories matching `*-loom`
- `Knot.strand_dir` and `Knot.tie_off_dir` are required fields in the `Knot` entity and knot file frontmatter
- `Loom.source_dir` is removed — the loom directory is derived from the loom ID
- `POST /looms` creates the loom directory (`<rig>/<id>/`) and accepts knot definitions with required `strand_dir` and `tie_off_dir`
- On restart, looms are re-discovered from the rig directory with correct paths — no collision with state files

## Implementation Status: ⬜ Draft

## Existing Tests

| Test | What it covers | Status |
|------|---------------|--------|
| `loom_repository::tests::scan_empty_rig` | Empty rig returns no looms | ✅ Green — will need update for naming filter |
| `loom_repository::tests::scan_rig_with_one_loom` | One loom with one knot | ✅ Green — loom dir named `my-loom`, already matches filter |
| `loom_repository::tests::scan_rig_with_multiple_looms` | Two looms | ✅ Green — dirs named `loom-a`, `loom-b`, matches filter |
| `loom_repository::tests::scan_skips_invalid_knot_files` | Malformed knot files skipped | ✅ Green — no change needed |
| `loom_repository::tests::scan_parses_knot_definition_files` | Knot fields parsed from frontmatter | ✅ Green — will need update for required fields |
| `loom_repository::tests::scan_parses_per_knot_source_and_tieoff_dirs` | Per-knot source/tie-off dirs | ✅ Green — will need update for `strand_dir` |
| `loom_repository::tests::scan_per_knot_source_dir_resolved_to_external` | External dir resolution | ✅ Green — will need update for `strand_dir` |
| `loom_repository::tests::scan_multiple_knots_different_source_dirs` | Multiple knots, different dirs | ✅ Green — will need update for `strand_dir` |
| `loom_repository::tests::scan_knot_without_dirs_gets_none` | Knot without dirs gets None | ✅ Green — **will fail**: dirs are now required |
| `knot_file::tests::valid_knot_file_parse` | Valid knot file parses | ✅ Green — will need update for required fields |
| `knot_file::tests::knot_file_with_source_and_tieoff_dirs` | Knot with custom dirs | ✅ Green — will need update for `strand_dir` |
| `knot_file::tests::knot_file_with_only_source_dir` | Knot with only source dir | ✅ Green — **will fail**: `tie_off_dir` now required |
| `knot_file::tests::knot_file_empty_dir_values_treated_as_none` | Empty dir values → None | ✅ Green — **will fail**: dirs are required |
| `integration::tests::startup_discovers_looms` | Startup discovers looms | ✅ Green — loom named `my-loom`, matches filter |
| `integration::tests::startup_starts_watchers` | Watchers started at startup | ✅ Green — will need update for `strand_dir` |
| `integration::tests::startup_logs_knot_registration` | Loom-log and knot state | ✅ Green — will need update for `strand_dir` |
| `integration::tests::event_flows_through_pipeline` | Full pipeline with mock agent | ✅ Green — will need update for `strand_dir` |
| `integration::tests::full_pipeline_create_modify_delete` | Create/modify/delete flow | ✅ Green — will need update for `strand_dir` |
| `integration::tests::multiple_looms_independent` | Two looms independently | ✅ Green — dirs `loom-a`, `loom-b`, matches filter |
| `inbound::tests::post_loom_success` | `POST /looms` returns 201 | ✅ Green — **will need rewrite**: new request shape |
| `inbound::tests::post_loom_missing_source_dir` | Missing source_dir returns 400 | ✅ Green — **will be removed**: different validation |
| `inbound::tests::post_loom_duplicate_id` | Duplicate returns 409 | ✅ Green — no change needed |
| `inbound::tests::post_loom_starts_watcher` | Watcher started for source dir | ✅ Green — **will need update**: watches strand_dir |

## Test Gaps

- No test verifying that non-`-loom` directories are ignored during discovery
- No test verifying that looms with no valid knot files are skipped
- No test for `POST /looms` with missing required knot fields (`strand_dir`, `tie_off_dir`)
- No test for the loom directory being created by `POST /looms`
- No test verifying that `Knot.strand_dir` is required (no `None` allowed)
- No integration test for a loom with multiple knots having different `strand_dir` values

## Phases

### Phase 0: Domain — `strand_dir`/`tie_off_dir` required in Knot and KnotFile

**Hex Layer:** Domain

Changes:
- `Knot` entity: rename `source_dir: Option<PathBuf>` → `strand_dir: PathBuf` (required). `tie_off_dir: Option<PathBuf>` → `tie_off_dir: PathBuf` (required).
- `KnotFile` parser: rename `source-dir` frontmatter field → `strand-dir` (required). `tie-off-dir` (required).
- `KnotFileError`: add variants `MissingStrandDir` and `MissingTieOffDir`.
- `Loom` entity: remove `source_dir` field. The loom directory is derived from the loom ID and the rig base path.

```diff
  pub struct Knot {
      pub id: KnotId,
      pub agent_config: AgentConfig,
      pub prompt_template: PromptTemplate,
-     pub source_dir: Option<PathBuf>,
-     pub tie_off_dir: Option<PathBuf>,
+     pub strand_dir: PathBuf,
+     pub tie_off_dir: PathBuf,
  }
```

- [x] Write failing tests: `knot_file::tests::missing_strand_dir_returns_error`, `knot_file::tests::missing_tieoff_dir_returns_error`, `entities::tests::knot_construction_with_required_dirs`
- [x] Implement: rename fields in `Knot` entity, update `KnotFile` parser, add new error variants
- [x] Update existing domain tests: rename `source_dir` references to `strand_dir`, update knot construction to include required fields
- [x] All domain tests green

### Phase 1: Application — Ports and Use Cases

**Hex Layer:** Application (Ports + Use Cases)

Changes:
- `LoomRepository` port: `scan()` returns looms without `source_dir` (field removed).
- `RegisterLoom` use case: no longer receives `source_dir`. Creates loom directory on disk via `LoomLogPort::open()`. Watches `strand_dir` per knot.
- `DiscoverLooms` use case: delegates filtering to the repository (no change needed here — filtering is in the adapter).
- `ProcessStrand` use case: uses `knot.strand_dir` instead of `knot.source_dir.unwrap_or(loom.source_dir)`.
- Update all use case tests: rename `source_dir` → `strand_dir`, remove `Loom.source_dir` construction.

- [x] Write failing tests: update existing `DiscoverLooms`, `RegisterLoom`, `ProcessStrand` tests with new field names and required dirs
- [x] Implement: update port trait signatures, update use case implementations, update `ProcessStrand` to use `knot.strand_dir`
- [x] All application tests green

**Note:** `skill_integration::knots_and_looms_register_and_list` fails because `POST /looms` handler still uses the old `source_dir` scan flow — fixed in Phase 4.

### Phase 2: Outbound Adapters — Loom Repository with `-loom` Filter

**Hex Layer:** Outbound Adapters

Changes:
- `FileSystemLoomRepository::scan()`: filter directories by `*-loom` suffix. Skip directories not matching this pattern.
- `scan_knot_files()`: update to parse `strand-dir` instead of `source-dir`. Knots missing required fields are skipped (logged as warning).
- Resolve per-knot paths relative to project root (parent of rig) — unchanged logic, just renamed field.

- [x] Write failing tests: `scan_skips_non_loom_directories` (a `rig/output` directory is ignored), `scan_requires_strand_and_tieoff_dirs` (knot without dirs is skipped), `scan_ignores_loom_log_directory` (a `rig/<id>/` created by log port is ignored because it doesn't end in `-loom`)
- [x] Implement: add `-loom` suffix filter in `scan()`, update field name in `scan_knot_files()`, add validation for required fields
- [x] Update existing adapter tests: rename `source_dir` → `strand_dir`, add required dirs to test knot content
- [x] All outbound adapter tests green

**Note:** Integration test failures are pre-existing path resolution issues and are Phase 5 scope.

### Phase 3: Outbound Adapters — Loom Log and Tie-Off Sink

**Hex Layer:** Outbound Adapters

Changes:
- `FileSystemLoomLog`: no path change needed (loom-log still lives in `<rig>/<loom-id>/.loom-log`, which is fine since `<loom-id>` ends in `-loom`).
- `FileSystemTieOffSink`: no changes needed — uses `knot.tie_off_dir` which is renamed but same logic.
- `NotifyEventSource`: no changes needed — uses paths passed to `watch()`.

- [x] Verify existing tests still pass after domain changes propagate
- [x] All outbound adapter tests green

**No code changes needed.** FileSystemLoomLog, FileSystemTieOffSink, NotifyEventSource all work correctly with the renamed fields. The -loom naming convention prevents log directory collision.

### Phase 4: Inbound Adapter — `POST /looms` Handler Rewrite

**Hex Layer:** Inbound Adapter

Changes:
- `RegisterLoomRequest`: replace `source_dir: Option<String>` with `knots: Vec<KnotRequest>` where `KnotRequest` has required `strand_dir` and `tie_off_dir` plus knot definition fields (`name`, `agent_config`, `prompt_template`).
- Handler: on successful request, create `<rig>/<id>/` directory, write knot `.md` file(s) to the loom directory, then call `RegisterLoom` use case with the assembled loom.
- The loom's knots are loaded from the created knot files (same path as discovery), ensuring consistency between API registration and file-system discovery.
- `POST /looms/discover`: unchanged — still scans rig directory.

```rust
pub struct RegisterLoomRequest {
    pub id: String,          // loom ID (will create `<rig>/<id>/` — must end in `-loom`)
    pub knots: Vec<KnotRequest>,
}

pub struct KnotRequest {
    pub name: String,
    pub agent_config: AgentConfig,
    pub prompt_template: PromptTemplate,
    pub strand_dir: String,  // required
    pub tie_off_dir: String, // required
}
```

- [x] Write failing tests: `post_loom_creates_loom_directory`, `post_loom_writes_knot_files`, `post_loom_missing_strand_dir_returns_400`, `post_loom_missing_tieoff_dir_returns_400`, `post_loom_requires_knots`, `post_loom_id_must_end_in_loom`
- [x] Implement: new request types, directory creation, knot file writing, updated validation
- [x] Update existing inbound tests: adapt to new request shape, update watcher verification to check `strand_dir`
- [x] All inbound adapter tests green

**Also updated:** `skill_integration` tests to use new API contract (`knots` array with `strand_dir`/`tie_off_dir`).

### Phase 5: Integration Tests — Full Pipeline Verification

**Hex Layer:** Integration

Changes:
- Update all integration tests to use `strand-dir` in knot file content.
- Add integration test: `discovery_ignores_non_loom_directories` — rig contains both `*-loom` and non-`*-loom` directories; only `-loom` dirs are discovered.
- Add integration test: `api_register_then_discover_after_restart` — register loom via API, verify loom directory created with knot files, stop server, restart, verify loom re-discovered with same config.

- [x] Update existing integration test knot content to use `strand-dir` and `tie-off-dir`
- [x] Write new integration tests listed above (`discovery_ignores_non_loom_directories`, `api_register_then_discover_after_restart`)
- [x] Full integration test suite green (31/31 passing)

### Phase 6: Composition Root and Skills Update

**Hex Layer:** Composition Root + Skills

Changes:
- `lib.rs`: `run_startup()` unchanged — delegates to `DiscoverLooms` which now uses filtered repository.
- `AppConfig::default_config()`: unchanged.
- Update skills (`knot-init`, `knots-and-looms`, `knot-inspect`) to reference new API contract (new `RegisterLoomRequest` shape).
- Update OpenAPI schema annotations to match new types.

- [x] Verify composition root compiles and integration tests pass
- [x] Update skill files with new API shapes
- [x] Update `utoipa` schema annotations for new types
- [x] Full build and test suite green

## Notes

- The `-loom` naming convention is a hard requirement for auto-discovery. API-registered looms must also use IDs ending in `-loom` (validated in Phase 4).
- The loom directory (`<rig>/<id>/`) serves two purposes: it holds knot `.md` files and the `.loom-log`. The naming convention ensures state directories don't collide with non-loom directories.
- `Loom.source_dir` removal simplifies the model: the loom directory is always `<rig>/<loom-id>/`, and each knot independently defines its `strand_dir`. This eliminates the terminology collision entirely.
