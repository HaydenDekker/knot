# Plan: Rig Switching and Sharing

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

It implements **Story 9: Rig Switching and Sharing** — enabling users to switch between multiple rigs on the same project and share rigs with colleagues by packaging loom definitions (excluding derived state like tie-offs and logs).

## Problem

Knot currently accepts no meaningful CLI arguments (only `--version`). The rig directory is hardcoded to `./rig` in `AppConfig::default_config()`. A user cannot:

- Switch between multiple rigs on the same project (e.g. `rig-dev` → `rig-review`)
- Share a rig's loom definitions with colleagues without manually copying files
- Get helpful feedback when multiple rigs exist and an explicit choice is required

## Target

After this plan:

- `knot` (no args) — auto-discovers rigs matching `*-rig` in the current directory:
  - **Zero matches** — creates `rig/` directory and starts with it (existing behaviour, preserved)
  - **One match** — uses that rig (e.g. `dev-rig/`)
  - **Multiple matches** — refuses to start, logs the found rig names, and instructs the user to specify one explicitly
- `knot <rig-name>` — starts Knot with the named rig directory (`<rig-name>/`). The rig name must exist or will be created.
- `knot share <rig-name>` — packages the rig's looms and profiles into a `.zip` artifact in the current directory. Excludes tie-offs and logs (derived state).

## Implementation Status: ✅ Complete (2026-06-17)

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `server_startup_smoke.rs` | Server starts with custom `rig_dir` via `AppConfig` | ✅ Green — passes `rig_dir` directly |
| `rig_lifecycle.rs` | Multiple rigs via `AppConfig` with different paths | ✅ Green — creates separate temp dirs |
| `discovery.rs` | Loom discovery inside a rig | ✅ Green — constructs `AppConfig` with explicit `rig_dir` |
| `helpers.rs` | `spawn_server()` accepts any `AppConfig` | ✅ Green — test utility |
| `composition.rs` | `build_app_context` wiring | ✅ Green — uses `AppConfig::default_config()` |

All tests construct `AppConfig` directly with an explicit `rig_dir`. The CLI (`main.rs`) is the only code path that uses `AppConfig::default_config()` — and it currently accepts no arguments beyond `--version`.

## Test Gaps

- No test for CLI argument parsing (rig name, share command)
- No test for auto-discovery of `*-rig` directories
- No test for the "multiple rigs found" error case
- No test for rig packaging (share command)
- No integration test for switching between rigs via CLI

## Hexagonal Structure

This plan touches two hexagonal layers but does **not** introduce a new use case:

| Layer | What is built |
|-------|---------------|
| **Domain** | `RigDiscovery` enum + `discover_rigs()` pure function — no ports, no IO traits |
| **Composition Root** | CLI argument parsing in `main.rs`, `AppConfig::with_rig_dir()` constructor, share command (walk + zip) |

Rig discovery is a **pure domain function** — scan a directory for `*-rig` subdirectories and return an enum. No ports, no store, no IO traits needed. It runs in `main.rs` **before** any use cases are constructed.

The share command is composition-root logic (walk directories, write a zip). Not a use case either.

**Affected code:**

| Component | Layer | Change |
|-----------|-------|--------|
| `src/domain/rig_discovery.rs` (new) | Domain | `RigDiscovery` enum + `discover_rigs()` pure function |
| `src/main.rs` | Composition Root | Parse CLI args, call discovery, construct `AppConfig` |
| `src/server.rs` | Composition Root | `AppConfig::with_rig_dir()` convenience constructor |
| `Cargo.toml` | — | Add `zip` crate dependency |
| `tests/rig_discovery.rs` (new) | Domain tests | Unit tests for `discover_rigs()` |
| `tests/rig_cli.rs` (new) | Integration tests | CLI tests via `std::process::Command` |

**Unchanged — no modifications needed:**

| Use Case | Why unchanged |
|----------|---------------|
| `DiscoverLooms` | Receives `workspace: &Path` parameter — already parameterised |
| `ReloadConfig` | Holds `rig_dir: PathBuf` — set by `AppConfig` |
| `ProcessStrand` | Holds `rig_dir: PathBuf` — set by `AppConfig` |
| `ConfigEventHandler` | Derives `project_root` from `rig_path` — already parameterised |
| `RegisterLoom` / `UnregisterLoom` | Receive `Loom` or `LoomId` — rig-independent |
| `ListLooms` / `GetLoom` / `GetLoomActivity` / `GetKnotStatus` | Read from `LoomStore` — rig-independent |
| `ManageKnot` | In-memory store operations — rig-independent |

## Phases

### Phase 0: Domain — Failing Tests for Rig Discovery

- [x] Create `tests/rig_discovery.rs` with failing unit tests for `discover_rigs()`:
  - Zero `*-rig` directories → `RigDiscovery::None`
  - One `*-rig` directory → `RigDiscovery::Single(path)`
  - Two `*-rig` directories → `RigDiscovery::Multiple([path1, path2])`
  - Three or more `*-rig` directories → `RigDiscovery::Multiple(paths)`
  - Explicit name given → `RigDiscovery::Named(path)` regardless of other `-rig` dirs present
  - Non-rig directories ignored (e.g. `src/`, `rig/` (no suffix), `planning-loom/`)
- [x] Tests use `tempfile::TempDir` for isolated filesystem state — no shared state, no rig creation side effects
- [x] Tests exercise `domain::rig_discovery::discover_rigs()` directly — no CLI, no `AppConfig`, no server
- [x] Tests fail (function doesn't exist yet) → green light to implement

### Phase 1: Domain — Implement Rig Discovery

- [x] Add `domain/rig_discovery.rs` module with:

```rust
pub enum RigDiscovery {
    None,
    Single(PathBuf),
    Multiple(Vec<PathBuf>),
    Named(PathBuf),
}

pub fn discover_rigs(
    base_dir: &Path,
    explicit_name: Option<&str>,
) -> RigDiscovery
```

- [x] `discover_rigs` scans `base_dir` for immediate subdirectories matching `*-rig` suffix, returns the appropriate variant
- [x] When `explicit_name` is `Some(name)`, returns `Named(base_dir.join(name))` — no scanning needed
- [x] Re-export from `domain/mod.rs`
- [x] Phase 0 tests pass

### Phase 2: Composition Root — AppConfig Constructor

- [x] Add `AppConfig::with_rig_dir(rig_dir: PathBuf) -> Self` in `server.rs`
- [x] Verify `AppConfig::default_config()` unchanged (all existing tests still pass)
- [x] Verify `lib.rs` re-exports remain valid

### Phase 3: Composition Root — CLI Parsing and Auto-Discovery

- [x] Modify `src/main.rs` to parse CLI arguments using `std::env::args()`:
  - `knot` — no args, trigger auto-discovery
  - `knot <rig-name>` — named rig
  - `knot share <rig-name>` — package rig (stub — exits with error, Phase 4 implements)
  - `knot --version` / `knot -V` — existing version check (preserved)
  - `knot --help` — print usage
- [x] Wire auto-discovery into `main`:
  - Call `discover_rigs(cwd, None)`
  - On `None` → fall through to `AppConfig::default_config()` (creates `rig/`)
  - On `Single(path)` → `AppConfig::with_rig_dir(path)`
  - On `Multiple(paths)` → `eprintln!` the found names with usage hint, `std::process::exit(1)`
  - On `Named(path)` → `AppConfig::with_rig_dir(path)`
- [x] Build succeeds, all existing tests still pass

### Phase 4: Composition Root — Share Command

- [x] Add `zip` crate to `Cargo.toml`
- [x] Smoke test in `tests/rig_discovery.rs`: verify `zip::ZipWriter` can create an in-memory zip and `zip::ZipArchive` can read it back, confirming the crate wires correctly
- [x] Implement `share` in `main.rs`:
  - Walk rig directory, collect all `*-loom/` directories and `profiles/`
  - Write `<rig-name>-rig.zip` in current directory
  - Exclude `tie-offs/`, `.rig-log`, `.workspace-agent-config.yaml`
- [x] Build succeeds

### Phase 5: Integration Tests — End-to-End via `std::process::Command`

- [x] Create `tests/rig_cli.rs` with integration tests that invoke the `knot` binary:
  - Two rigs exist, no args → non-zero exit, stderr contains rig names
  - One rig exists, no args → server starts, health check succeeds
  - Named rig → server starts, correct looms loaded
  - `knot share dev-rig` → zip exists with looms + profiles, no tie-offs
  - `knot share` nonexistent rig → non-zero exit
  - `knot share` without rig name → non-zero exit
- [x] Each test uses `tempfile::TempDir` for isolation
- [x] All integration tests pass

### Phase 6: Error Handling and Polish

- [x] Named rig doesn't exist → create it (existing `run_startup` behaviour)
- [x] Share: rig doesn't exist → error with clear message
- [x] Share: rig has no looms → still produce valid zip
- [x] `--help` and unknown arg handling
- [x] Full test suite passes (`cargo test` + `cargo clippy`)

## Notes

- **No external CLI parsing crate needed.** The argument space is tiny (`<rig-name>` and `share <rig-name>`). `std::env::args()` is sufficient and avoids a dependency.
- **The `-rig` naming convention** for rig directories mirrors the existing `-loom` convention. A rig directory like `dev-rig/` is distinct from a loom directory like `planning-loom/` (which lives *inside* a rig).
- **Use case impact:** None of the existing use cases need signature changes. `rig_dir` is already a field on `AppConfig` and flows through `build_app_context()` into all adapters and use cases. The only change is *how* `AppConfig.rig_dir` is determined before `start_server()` is called.
- **Share command uses `zip` crate** (or `flate2` + manual zip structure). Keep it lightweight — just walk the rig tree and stream into a zip file. No compression beyond deflate is needed.
- **Zip excludes:** `tie-offs/`, `.rig-log`, `.workspace-agent-config.yaml` (user's local config), and any files not under a loom or profiles directory.
