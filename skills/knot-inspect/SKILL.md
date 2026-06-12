---
name: knot-inspect
description: "Inspect the current state of a Knot rig: list looms, examine loom details, view activity logs, check knot processing status. Read-only access to rig state via Knot's HTTP API. USE FOR: inspect rig, check rig status, view looms, list looms, inspect loom, loom status, knot status, check knot, view activity, loom activity, processing status, knot state, rig state, what looms exist, show looms, loom details. DO NOT USE FOR: creating looms (use knots-and-looms), deleting looms (use knots-and-looms), initialising a rig (use knot-init), triggering processing."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.1.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Inspect Skill

Inspect the current state of a Knot rig. This skill provides read-only
access to rig configuration, loom details, activity logs, and knot
processing status through Knot's HTTP API.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### Read-Only

This skill only reads state. It does not modify, create, or delete any
resources. Use `knot-init` or `knots-and-looms` for write operations.

### Progressive Disclosure

Start broad (rig overview), then drill down (specific loom, then specific
knot) based on user requests.

---

## Prerequisites

1. Knot must be running (use `knot-init` skill if not)

---

## Agent Workflow

### Inspect the Full Rig

When asked to show rig status:

1. **Check health**: Send `GET /health` to verify Knot is running.
   If unreachable, report: "Knot is not running. Use `knot-init` skill."

2. **Show rig configuration**: Send `GET /config/rig`.
   Report the agent CLI path and arguments.

3. **List looms**: Send `GET /looms`.
   Present a summary table:

   | Loom ID | Source Dir | Knot Count |
   |---------|-----------|------------|
   | `prd-review` | `project/prds` | 2 |

4. **If no looms**: Report "No looms are registered. Use the
   `knots-and-looms` skill to create looms."

### Inspect a Specific Loom

When asked about a specific loom (by ID):

1. **Get loom details**: Send `GET /looms/{id}`.
   - On `404`: Report "Loom `{id}` not found. Run `GET /looms` to see
     available looms."

2. **Show loom configuration**:
   - ID, source directory
   - List of knots (by name)

3. **List knots**: Send `GET /looms/{id}/knots` to get knot names.

4. **Get activity log**: Send `GET /looms/{id}/activity`.
   - On `404`: Report "No activity log found for this loom."
   - Present the activity entries in chronological order:
     - Loom started events
     - Knot registered events
     - Strand processed events (with any errors)

### Inspect a Specific Knot

When asked about a specific knot within a loom:

1. **Get knot status**: Send `GET /looms/{loom_id}/knots/{knot_name}`.
   - On `404`: Report "Knot `{knot_name}` not found in loom `{loom_id}`."

2. **Show knot state**:
   - Knot ID
   - Current processing status (`idle`, `processing`, `completed`, `failed`)
   - Last processed strand path
   - Tie-off output path (if produced)
   - Error message (if failed)
   - Last updated timestamp

### Inspect All Knot States

When asked to show status of all knots across all looms:

1. Send `GET /looms` to get the list of looms.
2. For each loom, send `GET /looms/{id}/knots` to get knot names.
3. For each knot, send `GET /looms/{id}/knots/{knot_name}` to get status.
4. Present a consolidated table:

   | Loom | Knot | Status | Last Strand | Error |
   |------|------|--------|-------------|-------|
   | `prd-review` | `goals-review` | `completed` | `goals.md` | — |
   | `prd-review` | `non-goals-review` | `failed` | `non-goals.md` | timeout |

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Verify Knot is running |
| `/config/rig` | GET | Read rig agent configuration |
| `/looms` | GET | List all registered looms |
| `/looms/{id}` | GET | Get loom details (knots included) |
| `/looms/{id}/activity` | GET | Get loom activity log |
| `/looms/{id}/knots` | GET | List knot names in a loom |
| `/looms/{id}/knots/{knot_name}` | GET | Get processing state of a knot |

### Response Schemas

**GET /config/rig** → `RigAgentConfig`:
```json
{
  "cli_path": "pi",
  "cli_args": []
}
```

**GET /looms** → `Array<LoomSummary>`:
```json
[
  {
    "id": {"0": "my-loom"},
    "source_dir": "src/docs",
    "knot_count": 2
  }
]
```

**GET /looms/{id}** → `Loom`:
```json
{
  "id": {"0": "my-loom"},
  "source_dir": "src/docs",
  "knots": [
    {
      "id": {"0": "review"},
      "agent_config": {
        "goal": "Review PRD goals",
        "provider": "openai",
        "model": "gpt-4o",
        "tools": []
      },
      "prompt_template": {
        "input_bundling": "full-file",
        "instructions": "Review the goals section."
      }
    }
  ]
}
```

**GET /looms/{id}/activity** → `Array<LoomEvent>`:
```json
[
  {
    "type": "loom_started",
    "loom_id": {"0": "my-loom"}
  },
  {
    "type": "knot_registered",
    "loom_id": {"0": "my-loom"},
    "knot_id": {"0": "review"}
  },
  {
    "type": "strand_processed",
    "loom_id": {"0": "my-loom"},
    "strand_path": "src/input.md",
    "error": null
  }
]
```

**GET /looms/{id}/knots/{knot_name}** → `KnotStatus`:
```json
{
  "knot_id": {"0": "review"},
  "state": {
    "knot_id": {"0": "review"},
    "event_type": "modified",
    "strand_path": "src/input.md",
    "tie_off_path": "tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md",
    "status": "completed",
    "error": null,
    "last_updated": "2026-06-03T12:00:00Z"
  }
}
```

### Processing Status Values

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

### Knot Event Types

| Type | Meaning |
|------|---------|
| `created` | A new strand was detected |
| `modified` | An existing strand was changed |
| `deleted` | A strand was removed |

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `GET /health` fails | Knot is not running. Suggest `knot-init` skill. |
| `GET /looms/{id}` returns 404 | Loom not found. List available looms. |
| `GET /looms/{id}/activity` returns 404 | No activity log. Loom may have no events yet. |
| `GET /looms/{id}/knots/{name}` returns 404 | Knot not found. List available knots in the loom. |
| Connection refused | Knot is not running. Suggest `knot-init` skill. |

---

## Quick Reference

```bash
# Check health
curl http://localhost:3000/health

# View rig configuration
curl http://localhost:3000/config/rig

# List all looms
curl http://localhost:3000/looms

# Get loom details (includes knots)
curl http://localhost:3000/looms/my-loom

# List knot names in a loom
curl http://localhost:3000/looms/my-loom/knots

# View loom activity log
curl http://localhost:3000/looms/my-loom/activity

# Check knot processing status
curl http://localhost:3000/looms/my-loom/knots/review

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig
2. **knots-and-looms skill** — create, modify, or delete looms

This skill provides visibility into rig state. Use knots-and-looms for
changes.
