---
name: knot-create
description: "Create looms, knots, and profiles by writing .md files directly. Knot auto-discovers looms (directories ending in `-loom`) and parses knot definition files (`.md` files inside loom directories). Profiles live in `rig/profiles/`. Use Knot's GET endpoints to verify state after file changes. USE FOR: create loom, add loom, new loom, delete loom, remove loom, modify loom, update loom, create knot, add knot, configure knot, loom CRUD, knot CRUD, loom management, knot management, create profile, agent profile, profile CRUD. DO NOT USE FOR: initialising a rig (use knot-init), inspecting state (use knot-inspect), triggering processing, running agent sessions."
license: MIT
metadata:
  author: Knot Team
  version: "3.0.0"
  compatibility: "Knot 0.3.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Create Skill

Create and manage looms, knots, and agent profiles by writing `.md` files
directly to disk. Knot auto-discovers changes through its file watcher —
no HTTP registration is needed. Use Knot's `GET` endpoints to verify
state after file changes.

A **loom** is a directory inside the rig whose name ends in `-loom`.
Knot discovers these directories automatically and parses `.md` knot
definition files inside them. Each **knot** references a shared
**agent profile** that provides the LLM provider, model, tools, and
system prompt.

**Knot API base URL:** `http://localhost:3000` (GET endpoints only)
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### File-First

All configuration is `.md` files with YAML frontmatter. Write files
directly to disk — Knot's file watcher picks up changes automatically.
No HTTP registration needed.

### Auto-Discovery

- Looms: any subdirectory of `rig/` ending in `-loom` is discovered.
- Knots: any `.md` file inside a loom directory is parsed as a knot.
- Profiles: any `.md` file in `rig/profiles/` is parsed as a profile.
  Profiles are read fresh from disk at processing time.

### Git-Friendly

All configuration is plain files tracked by git. Changes are visible
through diffs. No hidden state.

### Confirm Before Destructive Actions

Always confirm with the user before deleting a loom directory or profile
file. Summarise what will be removed.

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
- A **loom** is a directory inside `rig/` whose name ends in `-loom`
  (e.g. `prd-review-loom`). Knot discovers these automatically.
- A **knot** is a `.md` file with YAML frontmatter inside a loom
  directory. It references a shared **agent profile** via
  `agent-profile-ref` and defines task-specific direction via
  `prompt-template.instructions`. Each knot watches its own
  `strand-dir` for input files (strands).
- An **agent profile** is a `.md` file with YAML frontmatter stored in
  `rig/profiles/{name}.md`. Multiple knots can reference the same
  profile.

---

## Agent Workflow

### Create an Agent Profile

Profiles must exist before knots can reference them. Create a profile
first, then create knots that reference it.

1. **Gather required information** from the user:
   - `name`: Profile identifier (e.g. `fast`, `reviewer`, `coder`)
   - `provider`: AI provider (e.g. `openai`, `anthropic`, or a pi
     provider name like `llama-workhorse`)
   - `model`: Model identifier (e.g. `gpt-4o`, `qwen3-27b`)
   - `system_prompt`: The agent's system prompt/persona
   - `tools` (optional): List of tool names (e.g. `fs`, `web`)

2. **Check for existing profiles**: Send `GET /profiles` to list all
   profiles. If a profile with the same name exists, ask the user
   whether to overwrite.

3. **Write the profile file** to `rig/profiles/{name}.md`:
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
   - Ensure the `rig/profiles/` directory exists (create it if needed).
   - The `name` in frontmatter should match the filename stem.

4. **Verify creation**: Send `GET /profiles/{name}` to confirm the
   profile is discoverable by Knot.

5. **Report success**: "Profile `fast` created at `rig/profiles/fast.md`."

### Modify a Profile

When asked to modify a profile, edit the `.md` file directly:

1. **Read the existing profile**: Send `GET /profiles/{name}` to see
   current values. On `404`: Profile does not exist.

2. **Edit the file** at `rig/profiles/{name}.md` with updated
   frontmatter values. The markdown body (after the closing `---`)
   is preserved automatically when editing frontmatter.

3. **Verify the change**: Send `GET /profiles/{name}` to confirm the
   new values are picked up.

4. **Report what changed**.

### Delete a Profile

When asked to delete a profile:

1. **Confirm with the user**: Show the profile's current configuration
   by sending `GET /profiles/{name}`. Warn that knots referencing this
   profile will fail on next processing. Ask the user to confirm.

2. **Delete the file** at `rig/profiles/{name}.md`.

3. **Verify deletion**: Send `GET /profiles` to confirm the profile is
   gone.

4. **Report success**: "Profile `fast` deleted."

### List All Profiles

When asked to show all profiles:

1. Send `GET /profiles`.
2. Present a summary table with: Name, Provider, Model, Tools.

---

### Create a Loom with Knots

A loom is created by making a directory (ending in `-loom`) and writing
`.md` knot definition files inside it.

1. **Gather required information** from the user:
   - `id`: Loom identifier, must end in `-loom`
     (e.g. `prd-review-loom`, `docs-loom`)
   - At least one knot definition (see below)

2. **Check for duplicates**: Send `GET /looms` to list existing looms.
   If a loom with the same ID exists, ask the user whether to modify
   the existing loom or choose a different ID.

3. **Verify profiles exist**: For each knot's `agent_profile_ref`,
   send `GET /profiles/{name}` to confirm the profile exists.
   If missing, ask the user to create it first.

4. **Create the loom directory** at `rig/{id}/` (e.g. `rig/prd-review-loom/`).

5. **Write knot definition files** inside the loom directory.
   For a single knot named `goals-review`:
   ```markdown
   ---
   name: goals-review
   agent-profile-ref: fast
   strand-dir: "project/prds"
   prompt-template:
     input-bundling: "full-file"
     instructions: |
       Review the goals section for clarity and measurability.
   ---

   # Goals Review Knot

   This knot reviews the goals section of PRD documents.
   ```
   Write this to `rig/prd-review-loom/goals-review.md`.

6. **Verify registration**: Send `GET /looms/{id}` to confirm Knot has
   discovered the loom and its knots.

7. **Report success**: "Loom `prd-review-loom` created with 1 knot."

### Add a Knot to an Existing Loom

When asked to add a knot to an existing loom:

1. **Verify the loom exists**: Send `GET /looms/{id}`.

2. **Verify the profile exists**: Send `GET /profiles/{name}` for the
   knot's `agent_profile_ref`.

3. **Write the knot file** as `{knot-name}.md` inside the loom
   directory (e.g. `rig/prd-review-loom/non-goals-review.md`):
   ```markdown
   ---
   name: non-goals-review
   agent-profile-ref: fast
   strand-dir: "project/prds"
   prompt-template:
     input-bundling: "full-file"
     instructions: |
       Review the non-goals section.
   ---

   # Non-Goals Review Knot
   ```

4. **Verify**: Send `GET /looms/{id}/knots` to confirm the new knot
   appears in the list.

5. **Report success**: "Knot `non-goals-review` added to loom
   `prd-review-loom`."

### Modify a Knot

When asked to modify a knot, edit its `.md` file directly:

1. **Read the existing loom**: Send `GET /looms/{id}` to see current
   knots.

2. **Edit the file** at `rig/{loom-id}/{knot-name}.md` with updated
   frontmatter values.

3. **Verify**: Send `GET /looms/{id}` to confirm the changes are picked
   up.

4. **Report what changed**.

### Delete a Knot

When asked to delete a knot:

1. **Confirm with the user**: Show the loom's current knots via
   `GET /looms/{id}/knots`. Ask the user to confirm deletion.

2. **Delete the file** at `rig/{loom-id}/{knot-name}.md`.

3. **Verify**: Send `GET /looms/{id}/knots` to confirm the knot is gone.

4. **Report success**: "Knot `non-goals-review` deleted from loom
   `prd-review-loom`."

### Delete a Loom

When asked to delete a loom:

1. **Confirm with the user**: Show the loom's current configuration
   by sending `GET /looms/{id}`. Ask the user to confirm deletion.
   Note: this deletes the entire directory and all its knot files.

2. **Remove the loom directory** at `rig/{id}/`.

3. **Verify deletion**: Send `GET /looms` to confirm the loom is gone.

4. **Report success**: "Loom `prd-review-loom` deleted."

### List All Looms

When asked to show all looms:

1. Send `GET /looms` to list all discovered looms.
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
| `name` | **Yes** | Profile identifier (becomes the filename stem) |
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

## Verification Endpoints (GET Only)

All configuration is file-first. Use these `GET` endpoints to verify
state after writing or editing files.

| Endpoint | Purpose |
|----------|---------|
| `GET /looms` | List all discovered looms |
| `GET /looms/{id}` | Get loom details (knots included) |
| `GET /looms/{id}/knots` | List knot names in a loom |
| `GET /looms/{id}/activity` | Get loom activity log |
| `GET /looms/{id}/knots/{name}` | Get knot processing status |
| `GET /profiles` | List all agent profiles |
| `GET /profiles/{name}` | Get a profile by name |

### Response Schemas

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

**GET /looms/{id}/knots** → `Array<String>`:
```json
["goals-review", "non-goals-review"]
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

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `GET /looms/{id}` returns 404 | Loom not found. Directory may not end in `-loom`, or file watcher hasn't picked it up yet. |
| `GET /profiles/{name}` returns 404 | Profile file not found or has invalid frontmatter. Check `rig/profiles/{name}.md`. |
| Profile not found at processing time | Knot will fail with `ProfileNotFound` error. Check activity log via `GET /looms/{id}/activity`. |
| Knot file parse errors | Knot is skipped. Check `GET /looms/{id}/activity` for `KnotParseWarning` events. |
| Connection refused | Knot is not running. Suggest `knot-init` skill. |

---

## Quick Reference

```bash
# Create a profile (write file directly)
mkdir -p rig/profiles
cat > rig/profiles/fast.md << 'EOF'
---
name: fast
provider: openai
model: gpt-4o
system-prompt: |
  You are a fast reviewer.
---
# Fast Profile
EOF

# Create a loom with a knot (write files directly)
mkdir -p rig/prd-review-loom
cat > rig/prd-review-loom/goals-review.md << 'EOF'
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
prompt-template:
  input-bundling: "full-file"
  instructions: "Review the goals section."
---
# Goals Review
EOF

# Verify Knot has discovered the changes
curl http://localhost:3000/looms
curl http://localhost:3000/looms/prd-review-loom
curl http://localhost:3000/profiles

# Delete a knot (remove its file)
rm rig/prd-review-loom/goals-review.md

# Delete a profile (remove its file)
rm rig/profiles/fast.md

# Delete a loom (remove the directory)
rm -rf rig/prd-review-loom

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig (prerequisite for this skill)
2. **knot-inspect skill** — inspect loom activity and knot processing state

This skill (`knot-create`) manages the full loom, knot, and profile
lifecycle through direct file operations. Use knot-inspect for
monitoring and debugging.
