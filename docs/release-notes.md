# Release Notes

## v0.12.0

Current version. Knot is a local agent orchestration service that
watches directories for file changes and triggers AI agent sessions.

### Core Features

- **File-first configuration** — All configuration is `.md` files with
  YAML frontmatter. Git-trackable, diff-visible.
- **Auto-discovery** — Looms (`*-loom/` directories), knots (`.md`
  files in looms), and profiles (`rig/profiles/*.md`) are discovered
  automatically via file watching.
- **Agent profiles** — Define which LLM provider, model, tools, and
  system prompt to use. Profiles are read fresh from disk at processing
  time.
- **Knot processing** — Goal-seeking agents that read strands (input
  files), inspect current state, and apply minimal changes to reach a
  goal. Idempotent by design.
- **Tie-off output** — Append-only output files that record the complete
  processing history per knot.
- **Activity logging** — Per-loom activity logs and a rig-wide
  operational log (`rig/.rig-log`) in JSONL format.
- **HTTP API** — Full REST API for configuration and observability, with
  Swagger UI at `/swagger-ui`.
- **Debounced event processing** — File events are debounced to avoid
  triggering on partial writes.
- **Graceful shutdown** — Cooperative cascade shutdown that drains
  pending events and writes `LoomStopped` events.
- **Configurable timeouts** — Per-profile session timeouts with
  `TimeoutExceeded` event logging.
- **Bidirectional feedback loops** — Support for knots that form
  convergent loops, with status-gating and strand acknowledgement
  patterns.
