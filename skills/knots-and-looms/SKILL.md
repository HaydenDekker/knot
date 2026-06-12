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

Create and modify looms through Knot's HTTP API, and create, modify,
and delete knot definition files on disk.

A **loom** watches a source directory for file changes and processes them
through configured **knots**. Each knot is a `.md` file with YAML
frontmatter that defines an agent configuration and prompt template.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### Looms via HTTP API

Looms are registered and unregistered through Knot's HTTP API. Knot
manages loom state internally.

### Knots as Files

Knots are `.md` files with YAML frontmatter placed inside a loom
directory. Creating, editing, and deleting knots is done by writing
files directly — not through the HTTP API.

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

2. **Check for duplicates**: Send `GET /looms` to list existing looms.
   If a loom with the same ID exists, ask the user whether to modify the
   existing loom or choose a different ID.

3. **Register the loom**: Send `POST /looms` with JSON body:
   ```json
   {
     "id": "my-loom",
     "source_dir": "project/prds"
   }
   ```
   - Expected response: `201 Created` with body `{"registered": true}`
   - On `409 Conflict`: A loom with this ID already exists.
   - On `400 Bad Request`: Missing or invalid `source_dir`.

4. **Verify registration**: Send `GET /looms/{id}` to confirm the loom
   was created successfully.

5. **Report success**: "Loom `my-loom` registered, watching
   `project/prds`."

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
2. Present a summary table with: ID, source dir, knot count.

---

## Knot Definition Files

Knots are `.md` files with YAML frontmatter placed inside a loom
directory. Knot discovers them by scanning for `.md` files.

### Knot File Format

A knot file is markdown with YAML frontmatter delimited by `---`:

```markdown
---
name: my-knot
agent-config:
  goal: "Review document for completeness"
  provider: "openai"
  model: "gpt-4o"
  tools: []
source-dir: "app"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review this document.
---

# My Knot

Free-form markdown documentation about this knot.
```

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | **Yes** | Unique knot identifier (becomes the `KnotId`) |
| `agent-config.goal` | **Yes** | The agent's goal/instruction |
| `agent-config.provider` | **Yes** | AI provider (e.g. `openai`, `anthropic`) |
| `agent-config.model` | **Yes** | Model identifier (e.g. `gpt-4o`) |
| `agent-config.tools` | No | List of tool names (e.g. `fs`, `web`). Defaults to empty. |
| `source-dir` | No | Per-knot source directory. Relative paths resolve against the project root. Falls back to loom-level source dir if omitted. |
| `prompt-template.input-bundling` | **Yes** | How input is bundled (e.g. `full-file`) |
| `prompt-template.instructions` | **Yes** | Prompt instructions sent to the agent |

### Directory Resolution

- `source-dir` is **relative to the project root**
  (the directory containing the `rig/` folder).
- If omitted, the knot uses the loom-level source directory.
- Absolute paths are used as-is.
- Tie-off paths are static — they follow the pattern:
  `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`
- Example project layout:
  ```
  project_root/              ← source-dir resolves from here
  ├── app/                   ← source-dir: "app"
  └── rig/                   ← rig directory
      ├── tie-offs/          ← static tie-off directory
      │   └── my-loom/       ← tie-offs for this loom
      │       └── my-knot/   ← tie-off artifacts for this knot
      │           └── my-knot-tie-off.md
      └── my-loom/           ← loom (knot files live here)
          └── my-knot.md
  ```

### Create a Knot

When asked to create a new knot:

1. **Identify the target loom directory** (e.g. `.knots/my-loom/`).
2. **Check for existing knot files**: List `.md` files in the loom
directory. If a file with the same knot name already exists, ask the
user whether to overwrite or choose a different name.
3. **Gather required information** from the user:
   - `name`: Unique knot identifier
   - `goal`: The agent's goal
   - `provider`: AI provider
   - `model`: Model identifier
   - `instructions`: Prompt instructions
   - `source-dir` (optional): Directory to watch. If the knot watches
     a directory outside the loom, specify it here (e.g. `../../app`).
4. **Write the knot file** as `<name>.md` in the loom directory with
   proper YAML frontmatter.
5. **Report success**: "Knot `my-knot` created in `.knots/my-loom/`."

### Modify a Knot

When asked to modify a knot:

1. Read the existing `.md` file in the loom directory.
2. Update the frontmatter fields as requested.
3. Write the file back.
4. Report what changed.

### Delete a Knot

When asked to delete a knot:

1. Confirm with the user which knot file to remove.
2. Delete the `.md` file from the loom directory.
3. Report success: "Knot `my-knot` deleted."

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
  "source_dir": "string (required, path to watch)"
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
 └── Loom (source dir + knots)
      └── Knot (agent config + prompt template)
```

- A **loom** is identified by a unique string ID and watches one source
  directory. Tie-off output follows a static path pattern:
  `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`.
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
  -d '{"id":"my-loom","source_dir":"src/docs"}'

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
