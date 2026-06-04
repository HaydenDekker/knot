---
name: knots-and-looms
description: "Create, modify, and delete looms and knots via Knot's HTTP API. A loom watches a source directory and processes files through configured knots. This skill handles the full loom/knot lifecycle. USE FOR: create loom, add loom, new loom, delete loom, remove loom, modify loom, update loom, create knot, add knot, configure knot, loom CRUD, knot CRUD, loom management, knot management. DO NOT USE FOR: initialising a rig (use knot-init), inspecting state (use knot-inspect), triggering processing, running agent sessions."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.1.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knots and Looms Skill

Create, modify, and delete looms and knots through Knot's HTTP API.

A **loom** watches a source directory for file changes and processes them
through configured **knots**. Each knot defines an agent configuration and
prompt template.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### HTTP-Only Configuration

This skill interacts with Knot **exclusively through its HTTP API**. No
direct file manipulation for loom registration. Knot manages its own
state files on disk.

### Confirm Before Destructive Actions

Always confirm with the user before deleting a loom. Summarise what will
be removed.

---

## Prerequisites

1. Knot must be running (use `knot-init` skill if not)
2. A rig must be initialised (verified by `GET /config/rig` returning 200)

---

## Agent Workflow

### Create a Loom

When asked to create a new loom:

1. **Gather required information** from the user:
   - `id`: A unique loom identifier (e.g. `prd-review`, `docs-summary`)
   - `source_dir`: The source directory to watch (e.g. `project/prds`)
   - `tie_off_dir` (optional): Output directory for processed files
     (defaults to `output` if not specified)

2. **Check for duplicates**: Send `GET /looms` to list existing looms.
   If a loom with the same ID exists, ask the user whether to modify the
   existing loom or choose a different ID.

3. **Register the loom**: Send `POST /looms` with JSON body:
   ```json
   {
     "id": "my-loom",
     "source_dir": "project/prds",
     "tie_off_dir": "output/prds"
   }
   ```
   - Expected response: `201 Created` with body `{"registered": true}`
   - On `409 Conflict`: A loom with this ID already exists.
   - On `400 Bad Request`: Missing or invalid `source_dir`.

4. **Verify registration**: Send `GET /looms/{id}` to confirm the loom
   was created successfully.

5. **Report success**: "Loom `my-loom` registered, watching
   `project/prds`, output to `output/prds`."

### Delete a Loom

When asked to delete a loom:

1. **Confirm with the user**: Show the loom's current configuration
   by sending `GET /looms/{id}`. Ask the user to confirm deletion.

2. **Unregister the loom**: Send `DELETE /looms/{id}`.
   - Expected response: `204 No Content`
   - On `404 Not Found`: Loom does not exist (already deleted).

3. **Verify deletion**: Send `GET /looms` to confirm the loom is gone.

4. **Report success**: "Loom `my-loom` unregistered."

### Modify a Loom

Knot's API currently supports registration and deletion. To modify an
existing loom's configuration:

1. **Unregister the existing loom**: Send `DELETE /looms/{id}`.
2. **Re-register with new configuration**: Send `POST /looms` with the
   updated body.

> **Note:** For knot-level changes (agent config, prompt template), the
> knots are defined within the loom's directory structure. Knot discovers
> knots from loom definition files. If the user wants to modify knot
> configurations, direct them to update the loom's definition files and
> restart Knot, or unregister/re-register the loom with updated config.

### List All Looms

When asked to show all looms:

1. Send `GET /looms` to list all registered looms.
2. Present a summary table with: ID, source dir, tie-off dir, knot count.

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/looms` | GET | List all looms |
| `/looms` | POST | Register a new loom |
| `/looms/{id}` | GET | Get loom details |
| `/looms/{id}` | DELETE | Unregister a loom |
| `/looms/{id}/knots` | GET | List knots in a loom |

### Request/Response Schemas

**POST /looms** — Register a new loom

Request body (`RegisterLoomRequest`):
```json
{
  "id": "string (required, unique loom ID)",
  "source_dir": "string (required, path to watch)",
  "tie_off_dir": "string (optional, defaults to 'output')"
}
```

Responses:
- `201 Created`: `{"registered": true}`
- `400 Bad Request`: `{"error": "source_dir is required..."}`
- `409 Conflict`: `{"error": "loom 'x' already registered"}`

**GET /looms** — List all looms

Response: `Array<LoomSummary>`
```json
[
  {
    "id": {"0": "my-loom"},
    "source_dir": "src/docs",
    "tie_off_dir": "output/docs",
    "knot_count": 2
  }
]
```

**GET /looms/{id}** — Get loom details

Response: `Loom`
```json
{
  "id": {"0": "my-loom"},
  "source_dir": "src/docs",
  "tie_off_dir": "output/docs",
  "knots": []
}
```

**DELETE /looms/{id}** — Unregister a loom

Responses:
- `204 No Content`
- `404 Not Found`

**GET /looms/{id}/knots** — List knot names in a loom

Response: `Array<String>` (knot name identifiers)

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `POST /looms` returns 409 | Loom ID already exists. Ask user to choose different ID or modify existing. |
| `POST /looms` returns 400 | Invalid request body. Report specific error to user. |
| `DELETE /looms/{id}` returns 404 | Loom not found. Already deleted or never existed. |
| Connection refused | Knot is not running. Suggest `knot-init` skill. |

---

## Domain Model

```
Rig (top-level container)
 └── Loom (source dir + tie-off dir + knots)
      └── Knot (agent config + prompt template)
```

- A **loom** is identified by a unique string ID and watches one source
  directory, writing output to one tie-off directory.
- A **knot** defines how files are processed: which agent runs (provider,
  model, goal) and how input is bundled (prompt template).
- Knots are discovered from the loom's directory structure when Knot
  starts or when a loom is registered.

---

## Quick Reference

```bash
# List all looms
curl http://localhost:3000/looms

# Register a loom
curl -X POST http://localhost:3000/looms \
  -H "Content-Type: application/json" \
  -d '{"id":"my-loom","source_dir":"src/docs","tie_off_dir":"output/docs"}'

# Get loom details
curl http://localhost:3000/looms/my-loom

# List knots in a loom
curl http://localhost:3000/looms/my-loom/knots

# Unregister a loom
curl -X DELETE http://localhost:3000/looms/my-loom

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig (prerequisite for this skill)
2. **knot-inspect skill** — inspect loom activity and knot processing state

This skill manages loom and knot lifecycle. Use knot-inspect for
monitoring and debugging.
