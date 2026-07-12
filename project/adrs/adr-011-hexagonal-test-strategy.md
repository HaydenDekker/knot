# ADR-011: Hexagonal Test Strategy

**Date**: 2026-07-02
**Status**: Accepted

## Context

Knot's test suite had three interconnected problems:

1. **Mock identity race** — process-global `PATH`/`env::set_var()` manipulation caused tests to pick up each other's mock binaries under parallel execution. Worked around with `TEST_MUTEX` serialisation across 11 test files.
2. **Test suite too slow (~205s)** — full Knot runtime (notify watching, debounce, state writer, subprocess agent) started per test, with real delays (10s retry loops, 5s debounce windows).
3. **No test isolation** — every test shared mutable process state (env vars, temp paths, mutex locks), making flaky failures hard to reproduce.

The root cause is that the test suite was **composition-heavy**: most integration tests spun up the full Knot runtime and tested application logic *through* real adapters. This conflated two concerns — testing the business logic against trait contracts, and testing that the adapters and composition root wire correctly.

The existing ADR-001 ("Integration Test Server Pattern") solved the server lifecycle problem but established a pattern where every integration test boots the full application. This is expensive and makes parallel execution fragile.

## Decision

Adopt a **hexagonal test strategy**: test each layer against its boundary, and test composition once. The test suite splits into three tiers:

### Tier 1: Application Tests (the bulk)

Test application logic against **mock port implementations**. No I/O, no subprocesses, no file watching.

**Scope:** All use cases, debounce logic, retry loops, error handling, state transitions.

**Pattern:**

```rust
#[test]
fn process_strand_writes_tie_off_on_success() {
    let mut tie_off_sink = TrackingTieOffSink::default();
    let agent_runner = TrackingAgentRunner::new(AgentOutput::success("review done"));
    let mut loom_log = TrackingLoomLog::default();
    // ... other tracking mocks ...

    let usecase = ProcessStrand::new(
        Arc::new(agent_runner),
        Arc::new(tie_off_sink),
        Arc::new(loom_log),
        // ...
    );

    usecase.execute(event).await;

    // Assert on tracking mocks
    assert_eq!(tie_off_sink.writes.len(), 1);
    assert_eq!(tie_off_sink.writes[0].content, "review done");
}
```

**Isolation:** Each test creates its own mock instances. Tests share nothing — fully parallel, no mutex needed.

### Tier 2: Adapter Tests (one per adapter)

Test each outbound adapter's I/O contract in isolation against a `tempfile::tempdir()`.

**Scope:** One test module per adapter, verifying that the real adapter satisfies its trait contract.

| Adapter | What the test verifies |
|---------|----------------------|
| `PiStdioAgentRunner` | Subprocess spawns, stdin/stdout capture, exit code, timeout enforcement |
| `PiJsonAgentRunner` | JSON-L parsing, session ID extraction, `stopReason` filtering, timeout |
| `FileSystemTieOffSink` | Write, append (with delimiter + header), read_content, directory creation |
| `FileSystemLoomLog` | Open (create directory), JSONL append, read_all, idempotent open |
| `FileSystemStateWriter` | Atomic write (`.tmp` + `rename`), valid JSON, concurrent writes |
| `FileSystemLoomRepository` | Scan rig directory, parse knot files, save, handle parse warnings |
| `FileSystemAgentProfileRepository` | Profile CRUD from `.md` files, YAML frontmatter parsing |
| `NotifyEventSource` | Watch/unwatch via notify, event emission on file create/modify/delete |

**Isolation:** Each adapter test gets its own `tempfile::tempdir()`. For process-based adapters (`PiStdioAgentRunner`, `PiJsonAgentRunner`), the mock binary path is unique per test instance (e.g. `tempfile::tempdir().join("mock-pi")`), avoiding path collisions under parallel execution.

**Mock subprocess pattern for process-based adapters:**

```rust
fn make_mock_runner() -> PiStdioAgentRunner {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mock-pi");
    fs::write(&path, "#!/usr/bin/env bash\necho \"deterministic output\"\n").unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o755)).unwrap();
    PiStdioAgentRunner::with_cli_path(path.to_string_lossy().to_string())
    // `dir` is kept alive by the runner (stored in a PhantomData or similar)
    // or test function keeps `dir` in scope
}
```

### Tier 3: Composition Smoke Tests (2 tests)

Test the full application with **all real adapters** wired through the composition root, using `cli_path` injection to provide a deterministic mock agent.

**Scope:** Proves the composition root (`build_app_context()`) correctly wires all adapters and the full event pipeline (file watch → debounce → process → tie-off → state.json) works end-to-end.

**Pattern:**

```rust
#[tokio::test]
async fn composition_smoke_stdio() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = tmp.path().join("rig");
    fs::create_dir_all(&rig).unwrap();

    // Mock agent: deterministic output
    let mock = create_mock_pi(&rig, "review complete");

    // Write knot + profile + strand (real files)
    create_knot_file(&rig.join("review-loom"), "review");
    create_fast_profile(&rig);
    fs::write(rig.parent().unwrap().join("strands/test.md"), "content").unwrap();

    // Start Knot with injected mock agent path
    let config = AppConfig::with_rig_dir(rig.clone()).with_cli_path(mock);
    let _handle = start_knot(config);

    // Assert: pipeline completes, tie-off exists, state.json updated
    wait_for_state_field(&rig, "looms.0.knots.0.status", "completed");
    assert!(rig.join("tie-offs/review-loom/tie-off-review.md").exists());
}
```

**One per adapter variant:** `composition_smoke_stdio` (verifies `PiStdioAgentRunner` wiring) and `composition_smoke_json` (verifies `PiJsonAgentRunner` wiring).

**Isolation:** Unique `tempfile::tempdir()` per test. Mock agent path unique per test. Fully parallel.

### Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                    Test Suite Structure                       │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Tier 3: Composition Smoke (2 tests)                   │  │
│  │  Real adapters + mock agent via cli_path               │  │
│  │  Proves: wiring works, full pipeline completes          │  │
│  │  ┌────────────┐  ┌────────────┐  ┌───────────────┐    │  │
│  │  │ Notify     │→│ Debounce   │→│ ProcessStrand │    │  │
│  │  │ (real)     │  │ (real)     │  │ (real)        │    │  │
│  │  └────────────┘  └────────────┘  └───────┬───────┘    │  │
│  │     ┌─────────┐  ┌────────────┐          │             │  │
│  │     │LoomRepo │  │StateWriter │          │             │  │
│  │     │(real)   │  │(real)      │    ┌─────▼──────┐      │  │
│  │     └─────────┘  └────────────┘    │PiStdio/    │      │  │
│  │                                    │PiJson      │      │  │
│  │                                    │(real, mock │      │  │
│  │                                    │ cli_path)  │      │  │
│  │                                    └────────────┘      │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Tier 2: Adapter Tests (8 tests, one per adapter)      │  │
│  │  Each adapter + tempfile, all others mocked             │  │
│  │  Proves: adapter satisfies trait contract               │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Tier 1: Application Tests (~90 tests, the bulk)       │  │
│  │  All ports mocked (TrackingTieOffSink, TrackingAgent   │  │
│  │  Runner, TrackingLoomLog, etc.)                         │  │
│  │  Proves: business logic, error handling, retry loops   │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

### Implications for Design

- **`start_knot()` becomes a smoke-test helper** — no longer the entry point for most tests. Most tests call use cases directly with injected mocks.
- **Composition root gains `cli_path: Option<PathBuf>`** — the only injection point. Used by smoke tests and adapter tests to provide deterministic mock agents without process-global env vars.
- **All port mocks become `Arc<dyn PortTrait>`** — composition root accepts `Arc<dyn Trait>` for all ports. Application tests construct `AppContext` or use cases directly with mock implementations.
- **No `TEST_MUTEX` or `acquire_test_lock()`** — test serialisation is eliminated. Tier 1 tests share nothing. Tier 2 tests each own a `tempfile`. Tier 3 tests each own a `tempfile` + unique mock path.
- **No process-global `std::env::set_var()`** — mock helpers return paths, never manipulate environment variables.
- **Test suite target: <30s** — Tier 1 tests run in milliseconds (pure logic). Tier 2 tests run in <1s each (file I/O or subprocess spawn). Tier 3 tests run in ~5s each (full pipeline with fast debounce).

### Testing Strategy

**Application tests** use tracking mock implementations that record calls and their arguments:

```rust
/// Records all `write` and `append` calls for inspection.
#[derive(Default)]
pub struct TrackingTieOffSink {
    pub writes: std::sync::Mutex<Vec<TieOff>>,
}

impl TieOffSink for TrackingTieOffSink {
    fn write(&self, tie_off: TieOff) -> Result<(), PortError> {
        self.writes.lock().unwrap().push(tie_off);
        Ok(())
    }
    // ...
}
```

**Adapter tests** verify the contract against the trait:

```rust
#[test]
fn tieoff_sink_write_creates_file() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = FileSystemTieOffSink::new(tmp.path().to_path_buf());

    let tie_off = TieOff { content: "output".to_string(), path: ... };
    sink.write(tie_off).unwrap();

    assert!(tmp.path().join("tie-offs/loom/tie-off-knot.md").exists());
}
```

**Smoke tests** verify the full pipeline:

```rust
#[tokio::test]
async fn composition_smoke_stdio() {
    // tempfile rig + mock agent via cli_path + real adapters
    // create strand → assert tie-off exists + state.json updated
}
```

### Dependencies

- `tempfile` (dev-dependency, already present)
- `tokio` (already present)

## Consequences

### Positive

- **Fast full suite** — Tier 1 tests run in milliseconds. Total suite target: <30s (down from ~205s).
- **Fully parallel** — no shared mutable state between tests. No `TEST_MUTEX`, no `acquire_test_lock()`, no process-global env manipulation.
- **Clear responsibility** — each test tier has a single purpose: application logic, adapter contract, or composition wiring.
- **Sum-of-parts composition** — if every adapter satisfies its trait contract (Tier 2), and every use case works against mocks (Tier 1), the smoke test (Tier 3) only needs to prove the happy-path wiring. Composition bugs are wiring errors — the compiler catches most of them.
- **Easier debugging** — failures in Tier 1 pinpoint the application bug. Failures in Tier 2 pinpoint the adapter bug. Failures in Tier 3 pinpoint the composition bug.
- **Domain and application layers tested thoroughly** — no behaviour is only tested through the slow full pipeline. Edge cases, error paths, and retry logic are all covered by fast mock-based tests.

### Negative

- **Migration effort** — existing integration tests (~90) must be rewritten against mock ports. This is a significant refactor but is phased (see migration plan).
- **Less "end-to-end" coverage** — only 2 smoke tests exercise the full pipeline. Relies on the hexagonal principle: if components satisfy their contracts, composition works.
- **Composition bugs less visible** — subtle interaction bugs between adapters (e.g., StateWriter write order vs TieOffSink) are only caught by the smoke test's happy path. Mitigated by thorough adapter contract tests and application-level edge case tests.
- **`start_knot()` loses its role** — the helper that served as the integration test entry point for ~18 files becomes a niche helper used only by smoke tests. Most test infrastructure (debounce env vars, state polling helpers) becomes unused.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|-----------------|
| **Keep full pipeline per test, add per-test CLI path** | Still slow — every test starts full Knot runtime with debounce, file watching, state writer. Fixes the race but not the speed. |
| **Keep full pipeline, add `TEST_MUTEX` everywhere** | Serialises all tests — defeats the purpose of parallel test execution. ~205s × test count. |
| **Separate process per test (spawn `knot` binary)** | Slow startup per test, no shared state, cannot inspect internals. |
| **Only smoke tests + unit tests** | Loses the adapter contract verification. Unit tests mock ports; adapter tests verify real I/O. Without adapter tests, filesystem or subprocess bugs go undetected. |
| **Integration test per feature (current approach)** | Every feature tested through the full pipeline. Slow, hard to isolate failures, shared mutable state causes races. |

## References

- ADR-001: [Integration Test Server Pattern](adr-001-integration-test-server-pattern.md) — superseded
- ADR-005: [Skill Integration Testing](adr-005-skill-integration-testing.md) — skill subprocess tests remain outside this strategy
- `src/application/ports.rs` — port trait definitions and in-memory mock implementations
- `tests/helpers.rs` — current test helpers (to be replaced during migration)
