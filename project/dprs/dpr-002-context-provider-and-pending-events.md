# DPR-002: Context Provider Pattern and Pending Event Visibility

**Created:** 2026-07-15
**Plan:** 060 — Pending Event Visibility for Agent Producers
**Version:** 0.28.0

---

## Summary

The `ContextProvider` trait replaces the single `build_listener_context()` function with a composable, state-aware pipeline for building dynamic prompt context segments. Its first implementation (`AgentEventsContextProvider`) injects both event emission instructions and pending event visibility into producer knot prompts.

---

## The ContextProvider Trait

**File:** `domain/events.rs`

```rust
pub trait ContextProvider {
    fn build_context(&self, input: &BuildContext) -> String;
}
```

A single-method trait defined in the domain layer. The `BuildContext` struct carries all data providers need:

- `knot: Knot` — the knot currently being invoked
- `loom_id: LoomId` — the loom containing the current knot
- `all_knots: Vec<Knot>` — all registered knots (for inspecting consumer relationships)
- `rig_dir: PathBuf` — the rig directory (for reading filesystem state)

The domain defines the interface; concrete implementations live in the application layer where they have access to the filesystem and rig state. This is a deliberate hex boundary — the domain says *what* context providers do, the application layer says *how*.

---

## AgentEventsContextProvider

**File:** `application/usecases/context_providers.rs`

The first and currently only `ContextProvider` implementation. Combines two concerns:

### 1. Event Emission Instructions

Delegates to `build_listener_context()` (now an internal helper in `domain/events.rs`) which scans all knots' `strand_source` entries for `EventUri` subscriptions where the current knot is the producer. Produces a markdown section listing event types and their descriptions.

### 2. Pending Event Visibility

Scans `rig/tie-offs/{loom-id}/{event-id}/` directories for dispatched event files. For each file found:

- Filters by `target-knot` frontmatter field (only events dispatched FROM this producer)
- Extracts `event-id`, `description`, `timestamp`, and filename from YAML frontmatter
- Formats as a `## Pending Events` section prepended to the emission instructions

When no pending events exist, the section is omitted entirely (no empty noise).

### Frontmatter Parsing

The pending event scan reads dispatched event files which have YAML-style frontmatter:

```yaml
---
event-id: PlanCreated
target-knot: plan-creator
timestamp: 2026-07-14T10:00:00Z
description: Implementation plan for feature X
---
```

The `extract_frontmatter()` helper parses this into a `Vec<(String, String)>` — no YAML dependency needed. Each line is split on the first `:` to extract key-value pairs.

### Event ID Extraction

The emission instructions contain bullet points like `- \`PlanCreated\` — description`. The `extract_emitted_event_ids()` function parses these to know which event directories to scan.

---

## ProcessStrand Integration

**File:** `application/usecases/process_strand.rs`

The `execute()` method builds a `BuildContext` from the store and rig directory, then calls `AgentEventsContextProvider.build_context(&build_ctx)`. The result replaces the old `build_listener_context()` call.

The event enforcement path (detecting missing events in tie-off content) uses `build_ctx.all_knots` instead of the moved `all_knots` variable.

---

## Event Format — Required Fields

The prompt template was updated to document three required fields:

```markdown
---
event: <EventId>
description: <short summary of what happened>
timestamp: <ISO 8601 timestamp>
<additional fields as relevant>
---
```

The `timestamp` field is new. The dispatch adapter (`FileSystemEventDispatcher`) prefers the agent's timestamp from the payload (semantically meaningful — when the event occurred from the agent's perspective) and falls back to the system timestamp if absent. The `timestamp` key is excluded from payload iteration to avoid duplication in the frontmatter.

A "do not edit" guidance was added to the prompt: agents may not edit dispatched events, so if adjustment is needed they must emit a new event.

---

## File Structure

| File | Role |
|------|------|
| `domain/events.rs` | `BuildContext`, `ContextProvider` trait, `build_listener_context()` (internal helper) |
| `application/usecases/context_providers.rs` | `AgentEventsContextProvider`, frontmatter parsing, event ID extraction, formatting |
| `application/usecases/process_strand.rs` | Builds `BuildContext`, calls provider, wires into execution flow |
| `adapters/outbound/event_dispatcher.rs` | `build_event_file_content()` — prefers agent timestamp, skips `timestamp` in payload iteration |

---

## Test Strategy

18 new tests across domain and application layers:

- **Phase 0 (domain):** `BuildContext` field verification, no-op provider, trait composability
- **Phase 1 (application):** No listeners → empty, listeners without pending events, pending events with metadata extraction, multiple event types, missing dispatch directory, no description handling
- **Phase 2 (process_strand):** Pending events appear in prompt, no pending events when no files, event enforcement flow unchanged
- **Phase 3 (format):** Timestamp field in prompt, "do not edit" guidance, required fields documentation, agent timestamp preference, system timestamp fallback
- **Phase 4 (integration):** Producer re-enters and sees pending event, multiple event types see relevant pending events, event enforcement regression

All 668 tests pass.

---

## Notes

- The `ContextProvider` trait is deliberately minimal (single method) — it can expand naturally when additional context types are needed (e.g., strand history, previous tie-offs).
- The pending event scan reads ALL dispatched event files (they persist as records, not just unprocessed items). The agent uses the filename timestamp and description to assess recency and relevance.
- `build_listener_context()` remains in the domain layer as a public function — its existing tests still pass and it could be called directly by consumers that don't need pending event scanning.
