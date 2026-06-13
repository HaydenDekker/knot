# ADR-005: Skill Integration Testing

**Date**: 2026-06-13
**Status**: Accepted

## Context

Knot's configuration is file-first: profiles, looms, and knots are created by writing `.md` files directly. The Pi agent skills (`knot-init`, `knot-create`, `knot-inspect`) document the exact workflows an agent follows — write files, verify via GET endpoints, report results.

The unit test suite thoroughly covers the repository layer (profile_repo.rs: 16 tests, loom_repository.rs: 20 tests, knot_file.rs: 30 tests) and the inbound handler layer (loom.rs tests: 20+ tests). Integration tests cover the server-side file watcher pipeline (auto-discovery, knot lifecycle, pipeline processing).

However, there is no test that validates the **skill workflow end-to-end**: an agent following the skill instructions produces the expected result. This gap means:

1. Skill documentation may drift from actual server behaviour
2. File-first CRUD operations (especially loom deletion, profile modify/delete) lack integration-level verification
3. There is no safety net if a skill's file format or path assumptions become stale

## Decision

Skill integration tests invoke the **Pi CLI as a subprocess** to execute skill workflows against a running Knot server, then verify results through HTTP GET endpoints and filesystem checks.

### Architecture Overview

```
┌─────────────────────────────────────────────────┐
│                   Test Harness                   │
│                                                  │
│  1. Spin up Knot server (temp dir as rig)        │
│  2. Spawn `pi` subprocess with skill prompt      │
│  3. Wait for subprocess completion               │
│  4. Verify via:                                 │
│     - GET /profiles, GET /profiles/{name}        │
│     - GET /looms, GET /looms/{id}                │
│     - Filesystem checks (files exist, content)   │
│  5. Tear down server + temp dir                  │
└─────────────────────────────────────────────────┘
         │                    │
         ▼                    ▼
  ┌──────────┐        ┌──────────┐
  │  Knot     │        │   Pi     │
  │  Server   │        │  CLI     │
  │ (axum)    │        │ (skill   │
  │           │        │  agent)  │
  └──────────┘        └──────────┘
```

The test harness lives in `tests/skill_integration.rs` (or a new `tests/skill_e2e.rs` for subprocess-based tests to avoid polluting the mock-port tests already in `skill_integration.rs`).

### Test Structure

Each skill workflow test follows this pattern:

```rust
#[tokio::test]
async fn skill_knot_create_profile_workflow() {
    // 1. Setup: temp rig directory, running server
    let tmp = tempfile::tempdir().unwrap();
    let _server = spawn_server_with_rig(&tmp.path());
    wait_for_server(&server).await;

    // 2. Verify pre-condition: no profiles exist
    assert_empty_profiles(&server).await;

    // 3. Invoke skill via Pi CLI subprocess
    let skill_prompt = r#"
Create a profile named "fast" with:
- provider: openai
- model: gpt-4o
- system-prompt: "You are a reviewer."
Write it to rig/profiles/fast.md then verify via GET /profiles.
    "#;

    let mut child = Command::new("pi")
        .arg("--prompt")
        .arg(skill_prompt)
        .env("KNOT_API_URL", "http://localhost:PORT")
        .current_dir(tmp.path())
        .spawn()
        .unwrap();

    // 4. Wait for completion (with timeout)
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        child.wait_with_output(),
    )
    .await
    .expect("skill should complete within 2 minutes");

    assert!(output.status.success());

    // 5. Verify post-conditions
    assert_profile_exists(&server, "fast").await;
    assert_file_exists(tmp.path().join("rig/profiles/fast.md"));
}
```

### Implications for Design

- **Tests are slow.** Pi subprocess invocation involves agent reasoning, tool calls, and HTTP round-trips. These tests belong in a separate test target or gate (e.g., `cargo test --test skill_e2e` run only in CI, not on `cargo test`).
- **Tests are non-deterministic in timing.** The agent may take different paths to the same result. Assertions verify *outcome*, not *process*.
- **Tests depend on Pi being installed.** The test target should be `ignore`d by default and run only in environments where `pi` is available.
- **The skill prompt is the test input.** The exact prompt text captures the skill's expected usage — it is the test's specification.

### Testing Strategy

The skill integration tests validate:

| What | How |
|------|-----|
| Skill produces correct files | Filesystem assertions after skill completes |
| Files are discoverable by Knot | GET endpoint assertions (profiles, looms, knots) |
| File formats are correct | Parsing succeeds (no errors in GET responses) |
| Skill verifies its own work | Agent output contains verification steps |

What these tests do **not** validate:

| Not Tested | Reason |
|------------|--------|
| Agent reasoning quality | Unit tests cover the server logic; agent quality is a Pi concern |
| All skill paths | Test the happy path; error paths covered by unit tests |
| Concurrent skill invocations | Skills are single-agent workflows |

## Consequences

### Positive

- **Skill documentation stays accurate.** If a skill's file format or paths drift, the integration test fails.
- **Full file-first workflow verified.** Profiles, looms, and knots are created, modified, and deleted exactly as an agent would do it.
- **Confidence in agent-driven usage.** The actual mechanism users interact with (Pi agent + skills) is tested end-to-end.

### Negative

- **Slow test execution.** Pi subprocess tests take 30-120 seconds each. They must be gated separately from the fast test suite.
- **Environmental dependency.** Requires `pi` CLI and Knot running. Tests are `#[ignore]` by default.
- **Flakiness risk.** Agent output may vary; assertions must focus on deterministic outcomes (file existence, HTTP responses) not agent conversation content.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| **Mock the skill entirely** — simulate file writes in Rust tests | Doesn't validate the skill works with a real agent; defeats the purpose of integration testing |
| **Record/replay skill sessions** — capture agent interactions as fixtures | Brittle — agent output changes with model updates; hard to maintain |
| **Test skills only manually** — human reads skill, follows workflow | No automated safety net; regression risk on every skill update |
| **Unit-test the skill markdown** — parse SKILL.md and verify structure | Validates format, not behaviour; doesn't prove the workflow produces correct results |
| **Use a test agent (deterministic mock)** — replace Pi with a script that follows skill instructions | Loses the real-agent validation; adds maintenance burden for the mock agent |

## References

- Plan 26: [http-observability-only.md](../plans/http-observability-only.md) — identified test gaps including "no file-first skill tests"
- Knot skills: `.agents/skills/knot-init/`, `.agents/skills/knot-create/`, `.agents/skills/knot-inspect/`
- Existing skill integration tests: `tests/skill_integration.rs` (mock-port tests validating skill file structure and API contracts)
- ADR-001: [integration-test-server-pattern.md](adr-001-integration-test-server-pattern.md) — server spawning pattern used by these tests
