---
name: knot-init
description: "Initialise a Knot rig in the current directory. Detects if a rig exists, verifies Knot is running (or provides guidance to start it), and creates the rig directory structure. Verifies setup via GET /config/rig. USE FOR: init knot, knot init, setup knot, configure knot rig, start knot, initialise knot, knot configuration, rig init, rig setup. DO NOT USE FOR: creating looms, creating knots, inspecting loom state, modifying existing looms."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.1.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Init Skill

Initialise a Knot rig in the current working directory. This skill detects
whether a rig already exists, verifies that the Knot HTTP service is running,
and provides guidance for getting started.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### HTTP-Only Configuration

This skill interacts with Knot **exclusively through its HTTP API**. No
direct file manipulation. Knot manages its own files on disk.

### Idempotent

Safe to run multiple times. If a rig already exists, the skill reports
its current state instead of recreating it.

---

## Prerequisites

1. Knot must be compiled and available (e.g. via `cargo run` or installed binary)
2. Knot HTTP service must be running on `localhost:3000` (or configured port)

---

## Agent Workflow

When asked to initialise a Knot rig:

1. **Check if Knot is running**: Send `GET /health` to
   `http://localhost:3000/health`. Expected response: `200 OK` with body `ok`.

2. **If Knot is NOT running**:
   - Report that Knot is not reachable.
   - Provide guidance: "Start Knot with `cargo run` from the Knot project
     directory, or run the Knot binary."
   - Do NOT proceed further until the user confirms Knot is running.

3. **If Knot IS running**, check rig configuration:
   - Send `GET /config/rig` to `http://localhost:3000/config/rig`.
   - Expected response: `200 OK` with JSON body:
     ```json
     {
       "cli_path": "pi",
       "cli_args": []
     }
     ```
   - This confirms the rig configuration is loaded (defaults or custom).

4. **Check for existing looms**:
   - Send `GET /looms` to `http://localhost:3000/looms`.
   - If the response is an empty array `[]`, the rig has no looms yet.
     Report: "Rig is initialised but has no looms. Use the `knots-and-looms`
     skill to create looms."
   - If the response contains looms, report the existing loom IDs and
     suggest using the `knot-inspect` skill to examine them.

5. **Verify rig directory structure** (optional, for the user's awareness):
   - Knot stores state files in the working directory (where it was started).
   - Key directories: `.loom-log` (activity logs), knot state files.
   - These are managed by Knot automatically — the user does not create them.

6. **Report success**: Summarise the rig state including:
   - Knot service status (running)
   - Rig configuration (from `/config/rig`)
   - Number of registered looms
   - Next steps (create looms with `knots-and-looms` skill)

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Verify Knot is running |
| `/config/rig` | GET | Read rig agent configuration |
| `/looms` | GET | List registered looms |

### Expected Response Schemas

**GET /health** → `200` with plain text body `ok`

**GET /config/rig** → `200` with `RigAgentConfig`:
```json
{
  "cli_path": "pi",
  "cli_args": []
}
```

**GET /looms** → `200` with `Array<LoomSummary>`:
```json
[
  {
    "id": {"0": "my-loom"},
    "source_dir": "src/docs",
    "knot_count": 2
  }
]
```

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `GET /health` returns non-200 | Knot is not running. Provide start instructions. |
| Connection refused | Knot is not running. Provide start instructions. |
| `GET /config/rig` returns error | Rig config may be missing. Report to user. |
| `GET /looms` returns error | Unexpected error. Report to user with details. |

---

## Quick Reference

```bash
# Check if Knot is running
curl http://localhost:3000/health

# View rig configuration
curl http://localhost:3000/config/rig

# List looms
curl http://localhost:3000/looms

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

After initialisation, the workflow continues with:

1. **knots-and-looms skill** — create, modify, or delete looms and knots
2. **knot-inspect skill** — inspect rig, loom, and knot state

This skill prepares the rig. The other skills manage the content.
