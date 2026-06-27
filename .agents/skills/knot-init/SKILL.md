---
name: knot-init
description: "Initialise a Knot rig in the current directory. Detects if a rig exists, verifies Knot is running by checking `rig/state.json`, and creates the rig directory structure. If no profiles exist, creates a default profile by reading available models from ~/.pi/agent/models.json. Verifies setup by reading `rig/state.json`. USE FOR: init knot, knot init, setup knot, configure knot rig, start knot, initialise knot, knot configuration, rig init, rig setup. DO NOT USE FOR: creating looms, creating knots, inspecting loom state, modifying existing looms."
license: MIT
metadata:
  author: Knot Team
  version: "3.0.0"
  compatibility: "Knot 0.18.0+"
---

# Knot Init Skill

Initialise a Knot rig in the current working directory. This skill detects
whether a rig already exists, verifies that Knot is running by checking
for `rig/state.json`, creates the rig directory structure, and sets up a
default agent profile if none exist.

**State file:** `rig/state.json` (written every 5 seconds by Knot)

---

## Core Philosophy

### File-First Configuration

This skill writes configuration files directly to disk. Knot discovers
these files through its file watcher — no registration is needed.

### Idempotent

Safe to run multiple times. If a rig already exists, the skill reports
its current state instead of recreating it.

### Profile Discovery

When no profiles exist, this skill reads available models from the pi
agent configuration at `~/.pi/agent/models.json` to populate the
default profile with a real provider and model.

---

## Prerequisites

1. Knot must be compiled and available (e.g. via `cargo run` or
   installed binary)

---

## Agent Workflow

When asked to initialise a Knot rig:

1. **Check if Knot is running**: Check if `rig/state.json` exists.
   - If the file exists and is valid JSON, Knot is running and the rig
     is initialised.
   - If the file does not exist, Knot may not be running or the rig has
     not been initialised yet.

2. **If Knot is NOT running**:
   - Check if `rig/state.json` exists:
     - If it exists but is older than 10 seconds (check `updated_at`),
       Knot may be slow to start. Wait and re-check.
     - If it does not exist, report that Knot is not reachable.
   - Provide guidance: "Start Knot with `cargo run` from the Knot
     project directory, or run the Knot binary."
   - Do NOT proceed further until the user confirms Knot is running.

3. **If Knot IS running**, check rig state:
   - Read `rig/state.json`.
   - Extract `rig_path` to confirm the rig configuration is loaded
     (defaults or custom).

4. **Ensure rig directory structure exists**:
   - Create `rig/profiles/` if it doesn't exist.
   - These directories are managed on disk — Knot auto-discovers them.

5. **Agent adapter configuration** (`rig/.workspace-agent-config.yaml`):
   - Knot auto-creates this file on first startup if it doesn't exist.
   - It controls how Knot invokes the Pi agent. Default content:
     ```yaml
     # Rig-level agent configuration.
     #
     # agent-adapter: which adapter to use for Pi invocations.
     #   pi-stdio — plain text stdout (default, current behaviour)
     #   pi-json  — JSON-L stream with session ID + token usage capture
     #
     agent-adapter: pi-stdio
     ```
   - `pi-stdio` — plain text output, current behaviour. No metadata
     capture (session ID, token usage).
   - `pi-json` — JSON-L output, captures session ID and token usage.
     Required for session resume and invocation visibility features.
   - To switch: edit `rig/.workspace-agent-config.yaml`, change
     `agent-adapter` to `pi-json`, restart Knot.

6. **Check for existing profiles**:
   - Read `rig/state.json` and extract the `profiles` array.
   - If profiles exist, report the available profile names and skip to
     step 8.

7. **Create default profile** (only when no profiles exist):
   - Read available models from `~/.pi/agent/models.json` to determine
     a provider and model for the default profile.
   - The models.json file has this structure:
     ```json
     {
       "providers": {
         "provider-name": {
           "baseUrl": "...",
           "api": "openai-completions",
           "models": [
             { "id": "model-id", ... }
           ]
         }
       }
     }
     ```
   - Use the first available provider and its first model.
   - Write the default profile to `rig/profiles/default.md`:
     ```markdown
     ---
     name: default
     provider: <provider-name>
     model: <model-id>
     system-prompt: |
       You are a helpful AI assistant. Follow the instructions
       provided in each task.
     ---

     # Default Profile

     Auto-generated default profile.
     Provider and model sourced from ~/.pi/agent/models.json.

     To change this profile, edit the frontmatter above.
     To add more profiles, create additional .md files in this
     directory (e.g. fast.md, reviewer.md).
     ```
   - If `~/.pi/agent/models.json` does not exist or cannot be read,
     use placeholder values and document them in the body:
     ```markdown
     ---
     name: default
     provider: openai
     model: gpt-4o
     system-prompt: |
       You are a helpful AI assistant.
     ---

     # Default Profile

     Auto-generated default profile.

     WARNING: Could not read ~/.pi/agent/models.json.
     Provider and model are placeholders — edit this file to
     configure a real provider and model.
     ```

8. **Verify profile creation**:
   - Read `rig/state.json` (wait up to 5 seconds for the state writer
     to flush) and confirm at least one profile exists in the
     `profiles` array.
   - If created in step 6, confirm `default` appears in the list.

9. **Check for existing looms**:
   - Read `rig/state.json` and check the `looms` array.
   - If the array is empty `[]`, the rig has no looms yet. Report:
     "Rig is initialised but has no looms. Use the `knot-create` skill
     to create looms."
   - If the array contains looms, report the existing loom IDs and
     suggest using the `knot-inspect` skill to examine them.

10. **Report success**: Summarise the rig state including:
   - Knot service status (running)
   - Rig path (from `rig/state.json`)
   - Profiles available (from `rig/state.json`)
   - Number of registered looms
   - Next steps (create looms with `knot-create` skill)

---

## State File Schema

`rig/state.json` contains the current snapshot of rig state:

```json
{
  "rig_path": "/absolute/path/to/rig",
  "looms": [],
  "profiles": [
    {
      "name": "default",
      "provider": "llama-workhorse",
      "model": "qwen3-27b"
    }
  ],
  "updated_at": "2026-06-18T12:00:00Z"
}
```

The `updated_at` field is an ISO 8601 UTC timestamp. Use it to
determine if the state file is stale (older than ~10 seconds means
Knot may not be writing state).

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `rig/state.json` does not exist | Knot is not running or rig not initialised. Provide start instructions. |
| `rig/state.json` is invalid JSON | State file may be corrupt or partially written. Wait a moment and re-read. |
| `rig/state.json` `updated_at` is stale | Knot may have crashed. Provide restart instructions. |
| `rig/state.json` `rig_path` is empty | Rig config may be missing. Report to user. |
| `rig/state.json` `profiles` is empty | No profiles exist. Create default profile. |
| `~/.pi/agent/models.json` not found | Use placeholder provider/model. Document in profile body. |

---

## Quick Reference

```bash
# Start Knot
cargo run

# Check if Knot is running (state file exists and is fresh)
cat rig/state.json | python3 -m json.tool

# Check when state was last updated
cat rig/state.json | python3 -c "import sys,json; print(json.load(sys.stdin)['updated_at'])"

# View profiles
cat rig/state.json | python3 -c "import sys,json; [print(p['name'], p['provider'], p['model']) for p in json.load(sys.stdin)['profiles']]"

# View looms
cat rig/state.json | python3 -c "import sys,json; [print(l['id'], len(l['knots']), 'knots') for l in json.load(sys.stdin)['looms']]"
```

---

## Cross-Reference

After initialisation, the workflow continues with:

1. **knot-create skill** — create looms, knots, and profiles (file-first)
2. **knot-inspect skill** — inspect rig, loom, and knot state

This skill prepares the rig. The other skills manage the content.
