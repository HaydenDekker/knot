---
name: knot-create
description: "Create looms, knots, and profiles by writing .md files directly. Knot auto-discovers looms (directories ending in `-loom`) and parses knot definition files (`.md` files inside loom directories). Profiles live in `rig/profiles/`. Read `rig/state.json` to verify state after file changes. USE FOR: create loom, add loom, new loom, delete loom, remove loom, modify loom, update loom, create knot, add knot, configure knot, loom CRUD, knot CRUD, loom management, knot management, create profile, agent profile, profile CRUD. DO NOT USE FOR: initialising a rig (use knot-init), inspecting state (use knot-inspect), triggering processing, running agent sessions."
license: MIT
metadata:
  author: Knot Team
  version: "3.0.0"
  compatibility: "Knot 0.4.0+"
---

# Knot Create Skill

Create and manage looms, knots, and agent profiles by writing `.md` files
directly to disk. Knot auto-discovers changes through its file watcher —
no registration is needed. Read `rig/state.json` to verify state after
file changes.

A **loom** is a directory inside the rig whose name ends in `-loom`.
Knot discovers these directories automatically and parses `.md` knot
definition files inside them. Each **knot** references a shared
**agent profile** that provides the LLM provider, model, tools, and
system prompt.

**State file:** `rig/state.json` (written every 5 seconds by Knot)

---

## Core Philosophy

### File-First

All configuration is `.md` files with YAML frontmatter. Write files
directly to disk — Knot's file watcher picks up changes automatically.
No registration needed.

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
2. A rig must be initialised (verified by checking `rig/state.json`
   exists and contains a `rig_path`)

---

## Domain Model

```
Rig (`./rig/`, top-level container)
 ├── state.json              ← runtime state snapshot (auto-generated)
 ├── profiles/
 │     └── {name}.md         ← shared agent profiles
 ├── tie-offs/
 │     └── {loom-id}/
 │           ├── .loom-log   ← activity log
 │           └── {knot-name}/
 │                 └── {knot-name}-tie-off.md
 └── {name}-loom/            ← loom directory (must end in `-loom`)
      ├── {knot-name}.md     ← knot definition files
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
   - `profile_prompt`: The agent's system prompt/persona
   - `tools` (optional): List of pi tool names (e.g. `read`, `write`, `edit`, `bash`)
   - `timeout` (optional): Session timeout in seconds. If omitted,
     the runner's default of 300 seconds (5 minutes) is used.

2. **Check for existing profiles**: Read `rig/state.json` and check the
   `profiles` array. If a profile with the same name exists, ask the
   user whether to overwrite.

3. **Write the profile file** to `rig/profiles/{name}.md`:
   ```markdown
   ---
   name: fast
   provider: openai
   model: gpt-4o
   tools:
     - read
     - grep
     - find
     - ls
   profile-prompt: |
     You are a fast reviewer. Keep responses concise and direct.
   ---

   # Fast Profile

   Lightweight profile for quick reviews.
   ```

   For long-running tasks (e.g. code generation across many files),
   set a higher timeout:

   ```markdown
   ---
   name: coder
   provider: openai
   model: gpt-4o
   tools:
     - read
     - write
     - edit
     - bash
   timeout: 600
   profile-prompt: |
     You are a code generation agent. Take your time to be thorough.
   ---

   # Coder Profile

   Profile for long-running code tasks with extended timeout.
   ```
   - Ensure the `rig/profiles/` directory exists (create it if needed).
   - The `name` in frontmatter should match the filename stem.

4. **Verify creation**: Read `rig/state.json` (wait up to 5 seconds
   for the state writer to flush) and confirm the profile appears in
   the `profiles` array.

5. **Report success**: "Profile `fast` created at `rig/profiles/fast.md`."

### Modify a Profile

When asked to modify a profile, edit the `.md` file directly:

1. **Read the existing profile**: Read `rig/profiles/{name}.md` to see
   current values. If the file does not exist, the profile does not
   exist.

2. **Edit the file** at `rig/profiles/{name}.md` with updated
   frontmatter values. The markdown body (after the closing `---`)
   is preserved automatically when editing frontmatter.

3. **Verify the change**: Read `rig/state.json` and confirm the profile
   entry is present. Note: `profile_prompt` is not in the state file —
   verify by re-reading the profile file. `timeout` is included in
   state.

4. **Report what changed**.

### Delete a Profile

When asked to delete a profile:

1. **Confirm with the user**: Read `rig/profiles/{name}.md` to show the
   profile's current configuration. Warn that knots referencing this
   profile will fail on next processing. Ask the user to confirm.

2. **Delete the file** at `rig/profiles/{name}.md`.

3. **Verify deletion**: Read `rig/state.json` and confirm the profile
   no longer appears in the `profiles` array.

4. **Report success**: "Profile `fast` deleted."

### List All Profiles

When asked to show all profiles:

1. Read `rig/state.json` and extract the `profiles` array.
2. Present a summary table with: Name, Provider, Model, Timeout (show
   "default" for null/missing values).

---

### Create a Loom with Knots

A loom is created by making a directory (ending in `-loom`) and writing
`.md` knot definition files inside it.

1. **Gather required information** from the user:
   - `id`: Loom identifier, must end in `-loom`
     (e.g. `prd-review-loom`, `docs-loom`)
   - At least one knot definition (see below)

2. **Check for duplicates**: Read `rig/state.json` and check the
   `looms` array. If a loom with the same ID exists, ask the user
   whether to modify the existing loom or choose a different ID.

3. **Verify profiles exist**: For each knot's `agent_profile_ref`,
   read `rig/state.json` and check the `profiles` array for the name.
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
     instructions: |
       Review the goals section for clarity and measurability.
   ---

   # Goals Review Knot

   This knot reviews the goals section of PRD documents.
   ```
   Write this to `rig/prd-review-loom/goals-review.md`.

6. **Verify registration**: Read `rig/state.json` (wait up to 5 seconds
   for the state writer to flush) and confirm the loom and its knots
   appear in the `looms` array.

7. **Report success**: "Loom `prd-review-loom` created with 1 knot."

### Add a Knot to an Existing Loom

When asked to add a knot to an existing loom:

1. **Verify the loom exists**: Read `rig/state.json` and find the loom
   in the `looms` array.

2. **Verify the profile exists**: Read `rig/state.json` and check the
   `profiles` array for the knot's `agent_profile_ref`.

3. **Write the knot file** as `{knot-name}.md` inside the loom
   directory (e.g. `rig/prd-review-loom/non-goals-review.md`):
   ```markdown
   ---
   name: non-goals-review
   agent-profile-ref: fast
   strand-dir: "project/prds"
   prompt-template:
     instructions: |
       Review the non-goals section.
   ---

   # Non-Goals Review Knot
   ```

4. **Verify**: Read `rig/state.json` (wait up to 5 seconds) and confirm
   the new knot appears in the loom's `knots` array.

5. **Report success**: "Knot `non-goals-review` added to loom
   `prd-review-loom`."

### Modify a Knot

When asked to modify a knot, edit its `.md` file directly:

1. **Read the existing loom**: Read `rig/state.json` to see current
   looms and knots.

2. **Edit the file** at `rig/{loom-id}/{knot-name}.md` with updated
   frontmatter values.

3. **Verify**: Read `rig/state.json` (wait up to 5 seconds) and confirm
   the knot entry is present.

4. **Report what changed**.

### Delete a Knot

When asked to delete a knot:

1. **Confirm with the user**: Read `rig/state.json` to show the loom's
   current knots. Ask the user to confirm deletion.

2. **Delete the file** at `rig/{loom-id}/{knot-name}.md`.

3. **Verify**: Read `rig/state.json` (wait up to 5 seconds) and confirm
   the knot no longer appears in the loom's `knots` array.

4. **Report success**: "Knot `non-goals-review` deleted from loom
   `prd-review-loom`."

### Delete a Loom

When asked to delete a loom:

1. **Confirm with the user**: Read `rig/state.json` to show the loom's
   current configuration. Ask the user to confirm deletion.
   Note: this deletes the entire directory and all its knot files.

2. **Remove the loom directory** at `rig/{id}/`.

3. **Verify deletion**: Read `rig/state.json` (wait up to 5 seconds)
   and confirm the loom no longer appears in the `looms` array.

4. **Report success**: "Loom `prd-review-loom` deleted."

### List All Looms

When asked to show all looms:

1. Read `rig/state.json` and extract the `looms` array.
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
  - read
  - grep
  - find
  - ls
profile-prompt: |
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
| `tools` | No | List of pi tool names (e.g. `read`, `write`, `edit`, `bash`). Defaults to empty. Pi's built-in tools: `read`, `bash`, `edit`, `write`, `grep`, `find`, `ls`. |
| `profile-prompt` | **Yes** | The agent's system prompt/persona instructions |
| `timeout` | No | Session timeout in seconds. If omitted, the runner's default of 300 seconds (5 minutes) is used. When a session exceeds its timeout, a `TimeoutExceeded` event is recorded in the rig-log and the tie-off file is preserved unchanged. |

### How Profiles Are Used at Processing Time

When a strand event triggers a knot:

1. The knot's `agent-profile-ref` is used to load the profile from
   `rig/profiles/{name}.md` (read fresh from disk each time).
2. The profile provides: `provider`, `model`, `tools`.
3. The profile's `profile-prompt` is merged with the knot's
   `prompt-template.instructions` to form the full system prompt:
   ```
   {profile profile-prompt}

   {knot instructions}
   ```
4. This merged prompt is delivered via stdin to the agent runner (not via `--system-prompt`).

Edits to a profile file are picked up on the next strand event —
no restart needed.

---

## State File Schema

`rig/state.json` is the source of truth for current rig state. It is
written atomically every 5 seconds.

```json
{
  "rig_path": "/absolute/path/to/rig",
  "looms": [
    {
      "id": "prd-review-loom",
      "knots": [
        {
          "id": "goals-review",
          "status": "idle",
          "last_strand_path": null,
          "last_tie_off_path": null,
          "last_error": null,
          "last_event_at": null
        }
      ]
    }
  ],
  "profiles": [
    {
      "name": "fast",
      "provider": "openai",
      "model": "gpt-4o",
      "timeout": 600
    }
  ],
  "updated_at": "2026-06-18T12:00:00Z"
}
```

### Knot Status Values

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

> **Note:** The state file includes `name`, `provider`, `model`, and
> `timeout` for profiles but not `tools` or `profile_prompt`.
> To check those fields, read the profile file directly from
> `rig/profiles/{name}.md`.

---

## Error Handling

| Scenario | Action |
|----------|--------|
| Loom `{id}` not in `rig/state.json` | Directory may not end in `-loom`, or file watcher hasn't picked it up yet. Wait up to 5 seconds and re-check. |
| Profile `{name}` not in `rig/state.json` | Profile file not found or has invalid frontmatter. Check `rig/profiles/{name}.md`. |
| Profile not found at processing time | Knot will fail with `ProfileNotFound` error. Check activity log at `rig/tie-offs/{loom-id}/.loom-log`. |
| Knot file parse errors | Knot is skipped. Check `rig/tie-offs/{loom-id}/.loom-log` for `KnotParseWarning` events. |
| `rig/state.json` does not exist | Knot is not running. Suggest `knot-init` skill. |

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
profile-prompt: |
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
  instructions: "Review the goals section."
---
# Goals Review
EOF

# Verify Knot has discovered the changes
# Wait up to 5 seconds, then:
cat rig/state.json | python3 -m json.tool

# Delete a knot (remove its file)
rm rig/prd-review-loom/goals-review.md

# Delete a profile (remove its file)
rm rig/profiles/fast.md

# Delete a loom (remove the directory)
rm -rf rig/prd-review-loom
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig (prerequisite for this skill)
2. **knot-inspect skill** — inspect loom activity and knot processing state

This skill (`knot-create`) manages the full loom, knot, and profile
lifecycle through direct file operations. Use knot-inspect for
monitoring and debugging.
