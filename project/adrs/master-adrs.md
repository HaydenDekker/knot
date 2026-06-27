# Master ADR — Decision Index

> **Last Updated:** 2026-06-27

## ADR Index

| # | ADR | Status | Date |
|---|-----|--------|------|
| 9 | [ADR-009: Agent-Specific Adapters](adr-009-agent-specific-adapters.md) | Accepted | 2026-06-27 |
| 8 | [ADR-008: Full File-First State](adr-008-full-file-first-state.md) | Accepted | 2026-06-19 |
| 7 | [ADR-007: Stdin-Only Agent Invocation](adr-007-stdin-only-agent-invocation.md) | Accepted | 2026-06-16 |
| 6 | [ADR-006: File-First Configuration](adr-006-file-first-configuration.md) | Accepted | 2026-06-13 |
| 5 | [ADR-005: Skill Integration Testing](adr-005-skill-integration-testing.md) | Accepted | 2026-06-13 |
| 4 | [ADR-004: Shared Agent Profiles](adr-004-shared-agent-profiles.md) | Accepted | 2026-06-11 |
| 3 | [ADR-003: Channel-Cascade Shutdown](adr-003-channel-cascade-shutdown.md) | Accepted | 2026-06-07 |
| 2 | [ADR-002: Server Child Tasks](adr-002-server-child-tasks.md) | Accepted | 2026-06-07 |
| 1 | [ADR-001: Integration Test Server Pattern](adr-001-integration-test-server-pattern.md) | Accepted | 2026-06-07 |

## ADR Summaries

### 9. Agent-Specific Adapters

**Status:** Accepted
**Date:** 2026-06-27
**Summary:** Each adapter type is specific to a target agent + protocol (e.g. `PiStdio`, `PiJson`). The rig config selects an adapter — it does not configure invocation details like `cli_path` or `cli_args`.

### 8. Full File-First State

**Status:** Accepted
**Date:** 2026-06-19
**Summary:** Remove HTTP interface entirely; all state observation via `rig/state.json` written on a poll cycle.

### 7. Stdin-Only Agent Invocation

**Status:** Accepted
**Date:** 2026-06-16
**Summary:** Remove `--system-prompt` CLI flag; all agent prompt content delivered via stdin as a single user message.

### 6. File-First Configuration

**Status:** Accepted
**Date:** 2026-06-13
**Summary:** Configuration (profiles, looms, knots) is file-first — skills write files directly, Knot's file watcher auto-discovers changes.

### 5. Skill Integration Testing

**Status:** Accepted
**Date:** 2026-06-13
**Summary:** Skills are verified through integration tests that run the actual agent workflow against the Knot service.

### 4. Shared Agent Profiles

**Status:** Accepted
**Date:** 2026-06-11
**Summary:** Multiple knots can reference shared agent profiles stored as `rig/profiles/{name}.md` files.

### 3. Channel-Cascade Shutdown

**Status:** Accepted
**Date:** 2026-06-07
**Summary:** Cooperative shutdown pattern using channel sentinels and JoinSet drain.

### 2. Server Child Tasks

**Status:** Accepted
**Date:** 2026-06-07
**Summary:** Server owns child tasks via JoinSet; graceful cascade shutdown on signal.

### 1. Integration Test Server Pattern

**Status:** Accepted
**Date:** 2026-06-07
**Summary:** Integration tests spawn the Knot server as a child process and communicate via HTTP/file polling.
