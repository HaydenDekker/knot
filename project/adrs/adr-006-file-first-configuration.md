# ADR-006: File-First Configuration — HTTP Observability Only

**Date**: 2026-06-13
**Status**: Accepted

## Context

Knot originally enforced an "HTTP-only" constraint from its PRD: "Skills interact with Knot only via its HTTP interface — no direct file system access by the skills." This meant all configuration (creating profiles, looms, knots) went through POST/PATCH/DELETE endpoints.

In practice, this constraint added complexity without value:

1. **Files are the source of truth** — Knot reads profiles from disk, discovers knots from `.md` files, and watches directories for changes. The HTTP endpoints were thin wrappers around `fs::write` and `fs::read`.

2. **File-level metadata couldn't pass through JSON** — Profile files contain markdown body content (freeform text after the YAML frontmatter). To preserve this through HTTP, the body had to be threaded through the `AgentProfileRepository::save()` trait, every mock implementation, and all handler types.

3. **The file watcher already activates changes** — Knot's `NotifyEventSource` watches `rig/` and loom directories. File changes are picked up without any HTTP call. The file watcher is the activation mechanism; HTTP was just a redundant write path.

4. **Git tracks everything** — Writing `.md` files directly is naturally version-controlled. No state lives outside the file system.

The HTTP control endpoints (7 total: POST/DELETE/DELETE/PATCH/DELETE/POST/DELETE) were removed, leaving only read-only GET endpoints for observability.

## Decision

Configuration is **file-first**: profiles, looms, and knots are created by writing `.md` files directly to the rig directory. Knot's file watcher auto-discovers all changes. The HTTP interface provides **observability only** — read endpoints for inspecting state.

### Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                    Configuration Layer                    │
│                                                           │
│  Skills / Users                                           │
│     │                                                     │
│     ▼                                                     │
│  Write .md files ──────────────────────────────────────┐ │
│  (rig/profiles/*.md,                                    │ │
│   rig/*-loom/*.md)                                      │ │
│     │                                                    │ │
│     │ file system events                                 │ │
│     ▼                                                    │ │
│  ┌─────────────┐       ┌──────────────────┐              │ │
│  │ NotifyEvent  │──────▶│ ConfigEvent      │              │ │
│  │ Source       │       │ Handler          │              │ │
│  │ (file watch) │       │ (register/       │              │ │
│  └─────────────┘       │  update/remove)  │              │ │
│                        └──────────────────┘              │ │
│                              │                           │ │
│                              ▼                           │ │
│                        ┌──────────┐                      │ │
│                        │ AppStore  │ ◀── GET endpoints   │ │
│                        │ (state)   │    (observability)   │ │
│                        └──────────┘                      │ │
│                                                          │ │
│  HTTP Interface (GET only):                              │ │
│  /health, /config/rig, /looms, /looms/{id},              │ │
│  /looms/{id}/activity, /looms/{id}/knots,                │ │
│  /looms/{id}/knots/{name}, /profiles,                    │ │
│  /profiles/{name}, /agents/{dir}, Swagger UI             │ │
└──────────────────────────────────────────────────────────┘
```

### Implications for Design

- **No write endpoints** — The HTTP interface exposes only GET methods. All mutation happens through file writes.
- **File watcher is the activation path** — `NotifyEventSource` → `ConfigEvent` → `ConfigEventHandler` is the sole mechanism for configuration changes to take effect.
- **Skills write files directly** — The `knot-create` skill writes `.md` files to `rig/profiles/` and `rig/{name}-loom/` directories. It verifies results via GET endpoints only.
- **`AgentProfile.body` preserved through file I/O** — The markdown body of profile files is read directly from disk (`extract_body()` helper), not threaded through JSON. `ProfileResponse` serialises it for observability.
- **No `LoomRemoved` event** — Deleting a loom directory removes its knots via `KnotDeleted` events, but the loom shell persists in the store with 0 knots. This is an acceptable trade-off since loom directories are rarely deleted programmatically.

### Configuration

No new configuration properties. Existing rig directory structure unchanged:
- `rig/profiles/{name}.md` — agent profiles
- `rig/{name}-loom/{knot}.md` — knot definitions

### Testing Strategy

File-first CRUD operations are validated at the integration level in `tests/skill_e2e.rs`:
- **5 integration tests** (loom delete, profile modify, profile delete, profile body e2e, knot delete) — spawn a real Knot server in a temp directory, write files, verify via GET endpoints
- **2 skill workflow tests** (`#[ignore]`, `pi`-dependent) — invoke `pi` CLI subprocess to execute `knot-create` skill, verify files created and discoverable

See [ADR-005: Skill Integration Testing](adr-005-skill-integration-testing.md) for the testing approach.

## Consequences

### Positive

- **No HTTP wrapper complexity** — No POST/PATCH/DELETE handler code, no request DTOs, no JSON round-trip for file-level metadata.
- **File-level metadata preserved naturally** — Writing the full `.md` file (frontmatter + body) is a single `fs::write`. No threading through JSON, no trait changes.
- **Git-native workflow** — All configuration changes are file writes. Git tracks everything, diff tools work naturally.
- **Reduced attack surface** — No HTTP write endpoints to secure or validate.
- **Skills are simpler** — `knot-create` writes files directly instead of constructing JSON payloads and making HTTP calls.
- **Code removed** — 3600+ lines of handler code, request types, and tests eliminated.

### Negative

- **PRD constraint reversed** — The original "HTTP-only" constraint is abandoned. This is documented in the plan but means the PRD no longer matches implementation.
- **No `LoomRemoved` event** — Loom directory deletion removes knots but not the loom shell. Acceptable trade-off; loom deletion is rare.
- **File watcher dependency** — Configuration changes require the file watcher to be running. If a user writes files and then queries HTTP before the watcher processes the event, the response may be stale. Existing debounce logic mitigates this.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| **Keep HTTP endpoints as convenience** | Added 3600+ lines of handler code and tests for functionality identical to `fs::write`. Maintenance burden without value. |
| **Hybrid: HTTP for creation, file for metadata** | Complexity of maintaining two write paths. File watcher already handles creation. |
| **HTTP with file passthrough** — `POST /profiles` accepts full file content | Still requires handler code, request types, and JSON wrapping. File watcher is the real activation path. |
| **CLI tool instead of skills writing files** | Skills already have file I/O capability via Pi's tool calls. A dedicated CLI adds another artifact to maintain. |

## References

- [ADR-005: Skill Integration Testing](adr-005-skill-integration-testing.md) — testing approach for file-first workflows
- [PRD: AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md) — original PRD with "HTTP-only" constraint (now superseded by this ADR)
- Source: `src/adapters/inbound/loom.rs` — GET-only handlers
- Source: `src/adapters/inbound/router.rs` — reduced route set
- Source: `.agents/skills/knot-create/SKILL.md` — file-first skill workflow
