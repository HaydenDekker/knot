---
name: knot-inspect
description: "Inspect the current state of a Knot rig: list looms, examine loom details, view activity logs, check knot processing status, list agent profiles. Read-only access to rig state via Knot's HTTP API. USE FOR: inspect rig, check rig status, view looms, list looms, inspect loom, loom status, knot status, check knot, view activity, loom activity, processing status, knot state, rig state, what looms exist, show looms, loom details, list profiles, view profile, check profile. DO NOT USE FOR: creating looms (use knots-and-looms), deleting looms (use knots-and-looms), creating profiles (use knots-and-looms), initialising a rig (use knot-init), triggering processing."
license: MIT
metadata:
  author: Knot Team
  version: "2.0.0"
  compatibility: "Knot 0.2.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Inspect Skill

Inspect the current state of a Knot rig. This skill provides read-only
access to rig configuration, loom details, activity logs, knot
processing status, and agent profiles through Knot's HTTP API.

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
   Report the rig path, agent CLI path, and arguments.

3. **List looms**: Send `GET /looms`.
   Present a summary table:

   | Loom ID | Knot Count |
   |---------|-----------|
   | `prd-review-loom` | 2 |

4. **List profiles**: Send `GET /profiles`.
   Present a summary table:

   | Profile | Provider | Model |
   |---------|----------|-------|
   | `fast` | `openai` | `gpt-4o` |

5. **If no looms**: Report "No looms are registered. Use the
   `knots-and-looms` skill to create looms."

### Inspect a Specific Loom

When asked about a specific loom (by ID):

1. **Get loom details**: Send `GET /looms/{id}`.
   - On `404`: Report "Loom `{id}` not found. Run `GET /looms` to see
     available looms."

2. **Show loom configuration**:
   - ID
   - List of knots with their profile references and strand directories

3. **List knots**: Send `GET /looms/{id}/knots` to get knot names.

4. **Get activity log**: Send `GET /looms/{id}/activity`.
   - On `404`: Report "No activity log found for this loom."
   - Present the activity entries in chronological order:
     - `LoomStarted` events
     - `KnotRegistered` events
     - `KnotProcessing` events (with strand path)
     - `KnotCompleted` events (with strand and tie-off paths)
     - `KnotFailed` events (with error message)
     - `StrandProcessed` events (with error if any)

### Inspect a Specific Knot

When asked about a specific knot within a loom:

1. **Get knot status**: Send `GET /looms/{loom_id}/knots/{knot_name}`.
   - On `404`: Report "Knot `{knot_name}` not found in loom `{loom_id}`."

2. **Show knot state**:
   - Knot ID and Loom ID
   - Current processing status (`idle`, `processing`, `completed`, `failed`)
   - Last processed strand path
   - Last tie-off output path (if produced)
   - Error message (if failed)

### Inspect All Knot States

When asked to show status of all knots across all looms:

1. Send `GET /looms` to get the list of looms.
2. For each loom, send `GET /looms/{id}/knots` to get knot names.
3. For each knot, send `GET /looms/{id}/knots/{knot_name}` to get status.
4. Present a consolidated table:

   | Loom | Knot | Status | Last Strand | Error |
   |------|------|--------|-------------|-------|
   | `prd-review-loom` | `goals-review` | `completed` | `goals.md` | — |
   | `prd-review-loom` | `non-goals-review` | `failed` | `non-goals.md` | timeout |

### Inspect Profiles

When asked to list or view agent profiles:

1. **List all profiles**: Send `GET /profiles`.
   Present a summary table with: Name, Provider, Model, Tools.

2. **View a specific profile**: Send `GET /profiles/{name}`.
   - On `404`: Report "Profile `{name}` not found. Run `GET /profiles`
     to see available profiles."
   - Show: name, provider, model, tools, system_prompt.

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Verify Knot is running |
| `/config/rig` | GET | Read rig configuration |
| `/looms` | GET | List all looms |
| `/looms/{id}` | GET | Get loom details (knots included) |
| `/looms/{id}/activity` | GET | Get loom activity log |
| `/looms/{id}/knots` | GET | List knot names in a loom |
| `/looms/{id}/knots/{knot_name}` | GET | Get processing state of a knot |
| `/profiles` | GET | List all agent profiles |
| `/profiles/{name}` | GET | Get a profile by name |

### Response Schemas

**GET /config/rig** → `RigConfigResponse`:
```json
{
  "rig_path": "/absolute/path/to/rig",
  "cli_path": "pi",
  "cli_args": []
}
```

**GET /looms** → `Array<LoomSummary>`:
```json
[
  {
    "id": {"0": "prd-review-loom"},
    "knot_count": 2
  }
]
```

**GET /looms/{id}** → `Loom`:
```json
{
  "id": {"0": "prd-review-loom"},
  "knots": [
    {
      "id": {"0": "goals-review"},
      "agent_profile_ref": "fast",
      "prompt_template": {
        "input_bundling": "full-file",
        "instructions": "Review the goals section."
      },
      "strand_dir": "/absolute/path/to/project/prds"
    }
  ]
}
```

**GET /looms/{id}/activity** → `Array<LoomEvent>`:
```json
[
  {
    "LoomStarted": {
      "loom_id": {"0": "prd-review-loom"},
      "timestamp": "2026-06-10T12:00:00Z"
    }
  },
  {
    "KnotRegistered": {
      "loom_id": {"0": "prd-review-loom"},
      "knot_id": {"0": "goals-review"},
      "timestamp": "2026-06-10T12:00:01Z"
    }
  },
  {
    "KnotProcessing": {
      "loom_id": {"0": "prd-review-loom"},
      "knot_id": {"0": "goals-review"},
      "strand_path": {"0": "project/prds/goals.md"},
      "timestamp": "2026-06-10T12:00:02Z"
    }
  },
  {
    "KnotCompleted": {
      "loom_id": {"0": "prd-review-loom"},
      "knot_id": {"0": "goals-review"},
      "strand_path": {"0": "project/prds/goals.md"},
      "tie_off_path": {"0": "rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md"},
      "timestamp": "2026-06-10T12:00:03Z"
    }
  }
]
```

**GET /looms/{id}/knots** → `Array<String>`:
```json
["goals-review", "non-goals-review"]
```

**GET /looms/{id}/knots/{knot_name}** → `KnotStatus`:
```json
{
  "knot_id": {"0": "goals-review"},
  "loom_id": {"0": "prd-review-loom"},
  "status": "completed",
  "last_strand_path": {"0": "project/prds/goals.md"},
  "last_tie_off_path": {"0": "rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md"},
  "last_error": null
}
```

**GET /profiles** → `Array<ProfileResponse>`:
```json
[
  {
    "name": "fast",
    "provider": "openai",
    "model": "gpt-4o",
    "tools": ["fs"],
    "system_prompt": "You are a fast reviewer."
  }
]
```

**GET /profiles/{name}** → `ProfileResponse`:
```json
{
  "name": "fast",
  "provider": "openai",
  "model": "gpt-4o",
  "tools": ["fs"],
  "system_prompt": "You are a fast reviewer. Keep responses concise and direct."
}
```

### Processing Status Values

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

### Loom Event Types

| Type | Meaning |
|------|---------|
| `LoomStarted` | Loom began processing |
| `LoomStopped` | Loom stopped processing |
| `KnotRegistered` | A knot was registered |
| `KnotDeregistered` | A knot was removed |
| `KnotParseWarning` | Unknown YAML property in knot file |
| `KnotProcessing` | A knot started processing a strand |
| `KnotCompleted` | A knot finished successfully |
| `KnotFailed` | A knot failed with an error |
| `StrandProcessed` | A strand was processed (success or failure) |

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `GET /health` fails | Knot is not running. Suggest `knot-init` skill. |
| `GET /looms/{id}` returns 404 | Loom not found. List available looms. |
| `GET /looms/{id}/activity` returns 404 | No activity log. Loom may have no events yet. |
| `GET /looms/{id}/knots/{name}` returns 404 | Knot not found. List available knots in the loom. |
| `GET /profiles/{name}` returns 404 | Profile not found. List available profiles. |
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
curl http://localhost:3000/looms/prd-review-loom

# List knot names in a loom
curl http://localhost:3000/looms/prd-review-loom/knots

# View loom activity log
curl http://localhost:3000/looms/prd-review-loom/activity

# Check knot processing status
curl http://localhost:3000/looms/prd-review-loom/knots/goals-review

# List all agent profiles
curl http://localhost:3000/profiles

# Get a profile by name
curl http://localhost:3000/profiles/fast

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig
2. **knots-and-looms skill** — create, modify, or delete looms, knots, and profiles

This skill provides visibility into rig state. Use knots-and-looms for
changes.
