# Plan: Rename Workspace → Rig

## Problem

The term "workspace" is generic and breaks the rope/textile theme. The top-level container — the aggregation of multiple looms and knots — should use **rig** instead (a ship's rig is the complete interconnected system of ropes, lines, and running rigging).

This affects:
- Type name: `WorkspaceAgentConfig` → `RigAgentConfig`
- Struct fields: `workspace_config` → `rig_config`
- Method parameters: `workspace: &Path` → `rig: &Path`
- Error variant: `WorkspaceScanFailed` → `RigScanFailed`
- HTTP route: `/config/workspace` → `/config/rig`
- Config file: `.workspace-agent-config.yaml` → `.rig-agent-config.yaml`
- Test names, variable names, comments across source and test files
- Domain glossary and all documentation

## Target

All occurrences of "workspace" as a domain concept are replaced with "rig". The codebase compiles, all tests pass, and documentation is consistent. The rope/textile theme is now fully coherent: **rig → loom → knot → strand → tie-off**.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test | Location | What it covers | Status |
|------|----------|----------------|--------|
| `domain::value_objects::tests::workspace_agent_config_defaults` | `src/domain/value_objects.rs` | `WorkspaceAgentConfig` defaults and custom config | ✅ Green |
| `domain::value_objects::tests::workspace_agent_config_serialization` | `src/domain/value_objects.rs` | Serde round-trip of `WorkspaceAgentConfig` | ✅ Green |
| `adapters::outbound::loom_repository::tests::scan_empty_workspace` | `src/adapters/outbound/loom_repository.rs` | Scan returns empty vec for no looms | ✅ Green |
| `adapters::outbound::loom_repository::tests::scan_workspace_with_one_loom` | `src/adapters/outbound/loom_repository.rs` | Scan returns one loom | ✅ Green |
| `adapters::outbound::loom_repository::tests::scan_workspace_with_multiple_looms` | `src/adapters/outbound/loom_repository.rs` | Scan returns two looms | ✅ Green |
| `adapters::outbound::loom_repository::tests::scan_workspace_with_relative_path` | `src/adapters/outbound/loom_repository.rs` | Relative path resolution | ✅ Green |
| `adapters::outbound::loom_repository::tests::scan_workspace_with_absolute_path` | `src/adapters/outbound/loom_repository.rs` | Absolute path canonicalisation | ✅ Green |
| `application::usecases::tests::discover_looms_empty_workspace` | `src/application/usecases.rs` | Empty workspace discovery | ✅ Green |
| `application::ports::tests` (PortError Display) | `src/application/ports.rs` | `WorkspaceScanFailed` Display impl | ✅ Green |
| `integration::tests::app_loads_workspace_agent_config` | `tests/integration.rs` | Config loaded and served at `/config/workspace` | ✅ Green |
| 15+ integration tests with `workspace_config` field | `tests/integration.rs` | Full pipeline, error handling, external dirs | ✅ Green |

## Test Gaps

None — this is a rename refactor with no behavioural change. All existing tests cover the affected code. The risk is tests that reference the old names in assertions (e.g. error message strings containing "workspace scan failed").

## Phases

### Phase 0: Type Rename — `WorkspaceAgentConfig` → `RigAgentConfig`

Rename the type everywhere it appears. This is the core domain type, so it touches every layer.

- [x] `src/domain/value_objects.rs` — struct name, impl blocks, doc comments, test function names (`workspace_agent_config_defaults` → `rig_agent_config_defaults`, `workspace_agent_config_serialization` → `rig_agent_config_serialization`), test assertions
- [x] `src/domain/entities.rs` — re-export line
- [x] `src/application/usecases.rs` — import, type annotations, `ProcessStrand` struct field and constructor
- [x] `src/adapters/inbound/mod.rs` — import, `AppContext` field type
- [x] `src/lib.rs` — pub use re-export, `AppConfig` field type, `load_workspace_config()` return/param types, `build_app_context()` calls
- [x] `tests/integration.rs` — import, all `WorkspaceAgentConfig` constructor calls and type annotations
- [x] `cargo build` — verify compilation

### Phase 1: Structural Renames — fields, parameters, errors, routes, config file

Rename the structural identifiers that reference the workspace concept.

- [ ] `src/application/ports.rs`:
  - `PortError::WorkspaceScanFailed` → `PortError::RigScanFailed`
  - Display impl: `"workspace scan failed"` → `"rig scan failed"`
  - `LoomRepository::scan(workspace: &Path)` → `scan(rig: &Path)`
  - Trait doc comments: "scan a workspace" → "scan a rig"
  - Inline mock impl and tests
- [ ] `src/application/usecases.rs`:
  - `ProcessStrand` struct: `workspace_config` → `rig_config`
  - Constructor params: `workspace_config` → `rig_config`
  - Method params and local variables
  - Doc comments and string literals
- [ ] `src/adapters/outbound/loom_repository.rs`:
  - `scan(workspace: &Path)` → `scan(rig: &Path)`
  - Local variable `workspace` → `rig` throughout
  - Test function names: `scan_empty_workspace` → `scan_empty_rig`, `scan_workspace_with_one_loom` → `scan_rig_with_one_loom`, `scan_workspace_with_multiple_looms` → `scan_rig_with_multiple_looms`, `scan_workspace_with_relative_path` → `scan_rig_with_relative_path`, `scan_workspace_with_absolute_path` → `scan_rig_with_absolute_path`
  - Test variable names and comments
- [ ] `src/adapters/inbound/mod.rs`:
  - `AppContext.workspace_config` → `rig_config`
  - `get_workspace_config()` → `get_rig_config()`
  - Route: `/config/workspace` → `/config/rig`
  - Route registration and doc comments
  - Mock repository test param name
- [ ] `src/lib.rs`:
  - `AppConfig.workspace_config` → `rig_config`
  - `load_workspace_config()` → `load_rig_config()`
  - Config file: `.workspace-agent-config.yaml` → `.rig-agent-config.yaml`
  - `build_app_context()`: local variable names, comments
  - `AppContext` field access: `ctx.workspace_config` → `ctx.rig_config`
  - `start_event_pipeline()`: local variable names
  - `run_startup()` comments: "scan workspace" → "scan rig"
  - Doc comments throughout
- [ ] `cargo build` — verify compilation

### Phase 2: Integration Tests

Rename all references in the integration test file.

- [ ] `tests/integration.rs`:
  - Import: `WorkspaceAgentConfig` → `RigAgentConfig` (already renamed in Phase 0, but verify)
  - `AppConfig` constructor: `workspace_config` → `rig_config` field
  - Test function: `app_loads_workspace_agent_config` → `app_loads_rig_agent_config`
  - HTTP assertion: `/config/workspace` → `/config/rig`
  - Local variables: `workspace` → `rig` throughout (e.g. `let workspace = root.join("workspace")` → `let rig = root.join("rig")`)
  - Directory names used in tests: `"workspace"` → `"rig"` (e.g. `root.join("workspace")`)
  - `ctx.workspace_config` → `ctx.rig_config` assertions
  - Comments: "workspace" → "rig" in doc comments and inline comments
  - `assert_eq!` on `ctx.rig_config.cli_path` and `ctx.rig_config.cli_args`
- [ ] `cargo test` — verify all tests pass

### Phase 3: Documentation

Update all documentation files to use "rig" terminology.

- [ ] `project/domain-glossary.md`:
  - Add new `Rig` term (top-level container, aggregation of looms)
  - Update term relationships diagram: `Rig → Loom → Knot → ...`
  - Update any inline references to "workspace"
  - Update `Last Updated` date
- [ ] `project/plans/master-plan.md` — update `Last Updated` date
- [ ] `project/plans/file-watcher.md` — update references in notes (e.g. "scans workspace" → "scans rig", test name references)
- [ ] `project/plans/knot-domain-models.md` — update `WorkspaceAgentConfig` → `RigAgentConfig` references in notes
- [ ] `project/plans/loom-config-and-path-resolve.md` — update "workspace" references in test name notes and descriptions
- [ ] `project/plans/loom-discovery-and-state.md` — update `scan(workspace: &Path)` → `scan(rig: &Path)` in port table
- [ ] `project/plans/pi-agent-integration.md` — update `WorkspaceAgentConfig` references
- [ ] `project/plans/system-integration.md` — update `WorkspaceAgentConfig` references
- [ ] `project/prds/prd-knot-skills.md` — "initialise a Knot workspace" → "initialise a Knot rig", story titles and acceptance criteria
- [ ] `project/prds/prd-ai-driven-file-generation.md` — "workspace" references in problem statement and notes
- [ ] `project/prds/master-prd.md` — "workspace" references in overview

## Notes

- This is a **pure rename refactor** — no behavioural change. Every existing test should continue to pass after its references are updated.
- The config file name change (`.workspace-agent-config.yaml` → `.rig-agent-config.yaml`) is a breaking change for any user who has created this file. Knot handles this gracefully: if the file doesn't exist, defaults are used.
- The HTTP route change (`/config/workspace` → `/config/rig`) is also a breaking change for any external caller. Currently there are no known external consumers.
- The `base_dir` field on `AppConfig` is NOT renamed — it refers to the base directory path, not the domain concept. Only identifiers that directly reference the "workspace" domain concept are changed.
