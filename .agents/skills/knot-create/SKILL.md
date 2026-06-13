---
name: knot-create
description: "Create looms and knots via Knot's HTTP API. A loom watches strand directories through configured knots. Each knot references a shared agent profile that provides the LLM provider, model, tools, and system prompt. This skill holds the theory and application notes on creating looms and knots, including profile management, the domain model, and the full CRUD lifecycle. USE FOR: create loom, add loom, new loom, delete loom, remove loom, modify loom, update loom, create knot, add knot, configure knot, loom CRUD, knot CRUD, loom management, knot management, create profile, agent profile, profile CRUD. DO NOT USE FOR: initialising a rig (use knot-init), inspecting state (use knot-inspect), triggering processing, running agent sessions."
license: MIT
metadata:
  author: Knot Team
  version: "2.0.0"
  compatibility: "Knot 0.2.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Create Skill

Create looms and knots through Knot's HTTP API and file system. This skill
holds the theory and application notes for the full loom/knot/profile
lifecycle — creation, modification, and deletion.

A **loom** is a directory inside the rig (name ending in `-loom`) that
contains `.md` knot definition files. Each **knot** references a shared
**agent profile** that provides the LLM provider, model, tools, and
system prompt. The knot's prompt template provides task-specific
instructions appended to the profile's system prompt.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### Looms via HTTP API

Looms are registered through Knot's HTTP API. Knot manages loom state
internally and discovers knots from `.md` files in the loom directory.

### Knots as Files

Knots are `.md` files with YAML frontmatter placed inside a loom
directory. Knots can also be created, modified, and deleted via
HTTP endpoints that write the files and update the in-memory store.

### Profiles as Files

Agent profiles are `.md` files with YAML frontmatter stored in
`rig/profiles/`. They can be managed via HTTP endpoints. Profiles are
resolved at processing time — edits are picked up on the next strand
event without restart.

### Confirm Before Destructive Actions

Always confirm with the user before deleting a loom or profile.
Summarise what will be removed.

---

## Prerequisites

1. Knot must be running (use `knot-init` skill if not)
2. A rig must be initialised (verified by `GET /config/rig` returning 200)

---

## Domain Model

```
Rig (`./rig/`, top-level container)
 ├── profiles/
 │     └── {name}.md           ← shared agent profiles
 ├── tie-offs/
 │     └── {loom-id}/
 │           ├── .loom-log      ← activity log
 │           └── {knot-name}/
 │                 └── {knot-name}-tie-off.md
 └── {name}-loom/              ← loom directory (must end in `-loom`)
      ├── {knot-name}.md       ← knot definition files
      └── ...
```

- A **rig** is the top-level container for all looms and profiles.
- A **loom** is identified by a unique string ID (must end in `-loom`)
  and contains `.md` knot definition files.
- A **knot** references a shared **agent profile** via `agent-profile-ref`
  and defines task-specific direction via `prompt-template.instructions`.
  Each knot watches its own `strand-dir` for input files (strands).
- An **agent profile** is a shared entity storing provider, model, tools,
  and system prompt. Multiple knots can reference the same profile.

---

## Agent Workflow

### Create an Agent Profile

Profiles must exist before knots can reference them. Create a profile
first, then create knots that reference it.

1. **Gather required information** from the user:
   - `name`: Profile identifier (e.g. `fast`, `reviewer`, `coder`)
   - `provider`: AI provider (e.g. `openai`, `anthropic`)
   - `model`: Model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`)
   - `system_prompt`: The agent's system prompt/persona
   - `tools` (optional): List of tool names (e.g. `fs`, `web`)

2. **Check for existing profiles**: Send `GET /profiles` to list all
   profiles. If a profile with the same name exists, ask the user
   whether to overwrite.

3. **Create the profile**: Send `POST /profiles/{name}` with JSON body:
   ```json
   {
     "provider": "openai",
     "model": "gpt-4o",
     "tools": ["fs"],
     "system_prompt": "You are a fast reviewer. Keep responses concise and direct."
   }
   ```
   - Expected response: `201 Created` with body `{"created": true}`
   - On `400 Bad Request`: Invalid fields (empty provider, model, or system_prompt).

4. **Verify creation**: Send `GET /profiles/{name}` to confirm the
   profile was created successfully.

5. **Report success**: "Profile `fast` created."

### Modify a Profile

When asked to modify a profile:

1. **Read the existing profile**: Send `GET /profiles/{name}`.
   On `404`: Profile does not exist.

2. **Re-create with updated values**: Send `POST /profiles/{name}`
   with the updated body. This overwrites the profile file while
   preserving any custom markdown body.

3. **Report what changed**.

### Delete a Profile

When asked to delete a profile:

1. **Confirm with the user**: Show the profile's current configuration
   by sending `GET /profiles/{name}`. Warn that knots referencing this
   profile will fail on next processing. Ask the user to confirm.

2. **Delete the profile**: Send `DELETE /profiles/{name}`.
   - Expected response: `204 No Content`
   - On `404 Not Found`: Profile does not exist.

3. **Verify deletion**: Send `GET /profiles` to confirm the profile is gone.

4. **Report success**: "Profile `fast` deleted."

### List All Profiles

When asked to show all profiles:

1. Send `GET /profiles`.
2. Present a summary table with: Name, Provider, Model, Tools.

### Create a Loom

A loom is registered with its knots inline. Knots reference profiles
that must already exist.

1. **Gather required information** from the user:
   - `id`: Unique loom identifier, must end in `-loom`
     (e.g. `prd-review-loom`, `docs-loom`)
   - At least one knot definition (see below)

2. **Check for duplicates**: Send `GET /looms` to list existing looms.
   If a loom with the same ID exists, ask the user whether to modify
   the existing loom or choose a different ID.

3. **Verify profiles exist**: For each knot's `agent_profile_ref`,
   send `GET /profiles/{name}` to confirm the profile exists.
   If missing, ask the user to create it first.

4. **Register the loom**: Send `POST /looms` with JSON body:
   ```json
   {
     "id": "prd-review-loom",
     "knots": [
       {
         "name": "goals-review",
         "agent_profile_ref": "fast",
         "strand_dir": "project/prds",
         "prompt_template": {
           "input_bundling": "full-file",
           "instructions": "Review the goals section for clarity and measurability."
         }
       }
     ]
   }
   ```
   - Expected response: `201 Created` with body `{"registered": true}`
   - On `400 Bad Request`: Invalid loom ID (must end in `-loom`) or
     empty knots list.

5. **Verify registration**: Send `GET /looms/{id}` to confirm the loom
   was created successfully.

6. **Report success**: "Loom `prd-review-loom` registered with 1 knot."

### Add a Knot to an Existing Loom

When asked to add a knot to an existing loom:

1. **Verify the loom exists**: Send `GET /looms/{id}`.

2. **Verify the profile exists**: Send `GET /profiles/{name}` for the
   knot's `agent_profile_ref`.

3. **Create the knot**: Send `POST /looms/{id}/knots` with JSON body:
   ```json
   {
     "name": "non-goals-review",
     "agent_profile_ref": "fast",
     "strand_dir": "project/prds",
     "prompt_template": {
       "input_bundling": "full-file",
       "instructions": "Review the non-goals section."
     }
   }
   ```
   - Expected response: `201 Created` with body `{"created": true}`
   - On `409 Conflict`: A knot with this name already exists.

4. **Report success**: "Knot `non-goals-review` added to loom `prd-review-loom`."

### Modify a Knot

When asked to modify a knot:

1. **Read the existing loom**: Send `GET /looms/{id}` to see current knots.

2. **Update the knot**: Send `PATCH /looms/{id}/knots/{name}` with the
   full updated knot JSON body (same shape as create).
   - Expected response: `200 OK` with body `{"updated": true}`
   - On `404 Not Found`: Loom or knot does not exist.

3. **Report what changed**.

### Delete a Knot

When asked to delete a knot:

1. **Confirm with the user**: Show the loom's current knots via
   `GET /looms/{id}/knots`. Ask the user to confirm deletion.

2. **Delete the knot**: Send `DELETE /looms/{id}/knots/{name}`.
   - Expected response: `204 No Content`
   - On `404 Not Found`: Loom or knot does not exist.

3. **Report success**: "Knot `non-goals-review` deleted from loom `prd-review-loom`."

### Delete a Loom

When asked to delete a loom:

1. **Confirm with the user**: Show the loom's current configuration
   by sending `GET /looms/{id}`. Ask the user to confirm deletion.

2. **Unregister the loom**: Send `DELETE /looms/{id}`.
   - Expected response: `204 No Content`
   - On `404 Not Found`: Loom does not exist.

3. **Verify deletion**: Send `GET /looms` to confirm the loom is gone.

4. **Report success**: "Loom `prd-review-loom` unregistered."

### List All Looms

When asked to show all looms:

1. Send `GET /looms` to list all registered looms.
2. Present a summary table with: ID, Knot Count.

---

## Knot Definition File Format

Knots are `.md` files with YAML frontmatter inside a loom directory
(`rig/{loom-id}/`). Knot discovers them by scanning for `.md` files.

### Example Knot File

```markdown
---
name: prd-goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
---

# PRD Goals Review Knot

This knot reviews the goals section of PRD documents.
```

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | **Yes** | Unique knot identifier (becomes the `KnotId`) |
| `agent-profile-ref` | **Yes** | Name of the agent profile to use (must exist in `rig/profiles/{name}.md`) |
| `strand-dir` | **Yes** | Directory to watch for strand files. Resolved relative to the project root. |
| `prompt-template.input-bundling` | **Yes** | How input is bundled (e.g. `full-file`) |
| `prompt-template.instructions` | **Yes** | Task-specific instructions appended to the profile's system prompt |

### Directory Resolution

- `strand-dir` is **relative to the project root**
  (the directory containing the `rig/` folder).
- Absolute paths are used as-is.
- Tie-off paths are statically derived:
  `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`
- Example project layout:
  ```
  project_root/              ← strand-dir resolves from here
  ├── project/prds/          ← strand-dir: "project/prds"
  └── rig/                   ← rig directory
      ├── profiles/          ← shared agent profiles
      │   └── fast.md
      ├── tie-offs/          ← static tie-off directory
      │   └── prd-review-loom/
      │       ├── .loom-log
      │       └── prd-goals-review/
      │           └── prd-goals-review-tie-off.md
      └── prd-review-loom/   ← loom directory
          └── prd-goals-review.md
  ```

---

## Agent Profile File Format

Profiles are `.md` files with YAML frontmatter stored in
`rig/profiles/{name}.md`.

### Example Profile File

```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - fs
system-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

Lightweight profile for quick reviews.
```

### Profile Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | **Yes** | Profile identifier (becomes the filename) |
| `provider` | **Yes** | LLM provider (e.g. `openai`, `anthropic`) |
| `model` | **Yes** | Model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`) |
| `tools` | No | List of tool names (e.g. `fs`, `web`). Defaults to empty. |
| `system-prompt` | **Yes** | The agent's system prompt/persona instructions |

### How Profiles Are Used at Processing Time

When a strand event triggers a knot:

1. The knot's `agent-profile-ref` is used to load the profile from
   `rig/profiles/{name}.md` (read fresh from disk each time).
2. The profile provides: `provider`, `model`, `tools`.
3. The profile's `system-prompt` is merged with the knot's
   `prompt-template.instructions` to form the full system prompt:
   ```
   {profile system-prompt}

   {knot instructions}
   ```
4. This merged prompt is passed as `--system-prompt` to the agent CLI.

Edits to a profile file are picked up on the next strand event —
no restart needed.

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/looms` | GET | List all looms |
| `/looms` | POST | Register a new loom (with knots) |
| `/looms/{id}` | GET | Get loom details |
| `/looms/{id}` | DELETE | Unregister a loom |
| `/looms/{id}/knots` | GET | List knot names in a loom |
| `/looms/{id}/knots` | POST | Add a knot to a loom |
| `/looms/{id}/knots/{name}` | PATCH | Update a knot |
| `/looms/{id}/knots/{name}` | DELETE | Remove a knot from a loom |
| `/profiles` | GET | List all agent profiles |
| `/profiles/{name}` | GET | Get a profile by name |
| `/profiles/{name}` | POST | Create or update a profile |
| `/profiles/{name}` | DELETE | Delete a profile |

### Request/Response Schemas

**POST /looms** — Register a new loom

Request body:
```json
{
  "id": "prd-review-loom",
  "knots": [
    {
      "name": "goals-review",
      "agent_profile_ref": "fast",
      "strand_dir": "project/prds",
      "prompt_template": {
        "input_bundling": "full-file",
        "instructions": "Review the goals section."
      }
    }
  ]
}
```

Responses:
- `201 Created`: `{"registered": true}`
- `400 Bad Request`: Invalid loom ID (must end in `-loom`) or empty knots.

**POST /looms/{id}/knots** — Add a knot

Request body:
```json
{
  "name": "non-goals-review",
  "agent_profile_ref": "fast",
  "strand_dir": "project/prds",
  "prompt_template": {
    "input_bundling": "full-file",
    "instructions": "Review the non-goals section."
  }
}
```

Responses:
- `201 Created`: `{"created": true}`
- `404 Not Found`: Loom does not exist.
- `409 Conflict`: Knot with this name already exists.

**PATCH /looms/{id}/knots/{name}** — Update a knot

Request body: same shape as POST (full knot definition).

Responses:
- `200 OK`: `{"updated": true}`
- `404 Not Found`: Loom or knot does not exist.

**DELETE /looms/{id}/knots/{name}** — Remove a knot

Responses:
- `204 No Content`
- `404 Not Found`: Loom or knot does not exist.

**GET /looms** — List all looms

Response: `Array<LoomSummary>`
```json
[
  {
    "id": {"0": "prd-review-loom"},
    "knot_count": 2
  }
]
```

**GET /looms/{id}** — Get loom details

Response: `Loom`
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

**DELETE /looms/{id}** — Unregister a loom

Responses:
- `204 No Content`
- `404 Not Found`

**GET /looms/{id}/knots** — List knot names

Response: `Array<String>` (knot name identifiers)

**POST /profiles/{name}** — Create or update a profile

Request body:
```json
{
  "provider": "openai",
  "model": "gpt-4o",
  "tools": ["fs"],
  "system_prompt": "You are a fast reviewer."
}
```

Responses:
- `201 Created`: `{"created": true}`
- `400 Bad Request`: Empty provider, model, or system_prompt.

**GET /profiles** — List all profiles

Response: `Array<ProfileResponse>`
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

**GET /profiles/{name}** — Get a profile

Response: `ProfileResponse` (same shape as above)

**DELETE /profiles/{name}** — Delete a profile

Responses:
- `204 No Content`
- `404 Not Found`

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `POST /looms` returns 400 | Invalid loom ID (must end in `-loom`) or empty knots. Report to user. |
| `POST /looms/{id}/knots` returns 409 | Knot name already exists. Ask user to choose different name. |
| `DELETE /looms/{id}` returns 404 | Loom not found. Already deleted or never existed. |
| `DELETE /profiles/{name}` returns 404 | Profile not found. |
| Profile not found at processing time | Knot will fail with `ProfileNotFound` error. Check `GET /looms/{id}/knots/{name}` status. |
| Connection refused | Knot is not running. Suggest `knot-init` skill. |

---

## Quick Reference

```bash
# List all looms
curl http://localhost:3000/looms

# Register a loom with knots
curl -X POST http://localhost:3000/looms \
  -H "Content-Type: application/json" \
  -d '{
    "id": "prd-review-loom",
    "knots": [{
      "name": "goals-review",
      "agent_profile_ref": "fast",
      "strand_dir": "project/prds",
      "prompt_template": {
        "input_bundling": "full-file",
        "instructions": "Review the goals section."
      }
    }]
  }'

# Get loom details
curl http://localhost:3000/looms/prd-review-loom

# List knots in a loom
curl http://localhost:3000/looms/prd-review-loom/knots

# Add a knot
curl -X POST http://localhost:3000/looms/prd-review-loom/knots \
  -H "Content-Type: application/json" \
  -d '{
    "name": "non-goals-review",
    "agent_profile_ref": "fast",
    "strand_dir": "project/prds",
    "prompt_template": {
      "input_bundling": "full-file",
      "instructions": "Review the non-goals section."
    }
  }'

# Update a knot
curl -X PATCH http://localhost:3000/looms/prd-review-loom/knots/goals-review \
  -H "Content-Type: application/json" \
  -d '{...}'

# Delete a knot
curl -X DELETE http://localhost:3000/looms/prd-review-loom/knots/goals-review

# Unregister a loom
curl -X DELETE http://localhost:3000/looms/prd-review-loom

# List all profiles
curl http://localhost:3000/profiles

# Create a profile
curl -X POST http://localhost:3000/profiles/fast \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "openai",
    "model": "gpt-4o",
    "system_prompt": "You are a fast reviewer."
  }'

# Get a profile
curl http://localhost:3000/profiles/fast

# Delete a profile
curl -X DELETE http://localhost:3000/profiles/fast

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig (prerequisite for this skill)
2. **knot-inspect skill** — inspect loom activity and knot processing state

This skill (`knot-create`) manages the full loom, knot, and profile lifecycle.
Use knot-inspect for monitoring and debugging.
