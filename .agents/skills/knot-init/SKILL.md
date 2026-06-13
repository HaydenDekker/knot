---
name: knot-init
description: "Initialise a Knot rig in the current directory. Detects if a rig exists, verifies Knot is running (or provides guidance to start it), and creates the rig directory structure. If no profiles exist, creates a default profile by reading available models from ~/.pi/agent/models.json. Verifies setup via GET /config/rig and GET /profiles. USE FOR: init knot, knot init, setup knot, configure knot rig, start knot, initialise knot, knot configuration, rig init, rig setup. DO NOT USE FOR: creating looms, creating knots, inspecting loom state, modifying existing looms."
license: MIT
metadata:
  author: Knot Team
  version: "2.0.0"
  compatibility: "Knot 0.2.0+"
  api_spec: "http://localhost:3000/swagger-ui/openapi.json"
---

# Knot Init Skill

Initialise a Knot rig in the current working directory. This skill detects
whether a rig already exists, verifies that the Knot HTTP service is
running, creates the rig directory structure, and sets up a default
agent profile if none exist.

**Knot API base URL:** `http://localhost:3000`
**OpenAPI spec:** `http://localhost:3000/swagger-ui/openapi.json`

---

## Core Philosophy

### File-First Configuration

This skill writes configuration files directly to disk. Knot discovers
these files through its file watcher — no HTTP registration is needed.
The HTTP API is used only for verification (GET endpoints).

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
2. Knot HTTP service must be running on `localhost:3000` (or configured
   port)

---

## Agent Workflow

When asked to initialise a Knot rig:

1. **Check if Knot is running**: Send `GET /health` to
   `http://localhost:3000/health`. Expected response: `200 OK` with
   body `ok`.

2. **If Knot is NOT running**:
   - Report that Knot is not reachable.
   - Provide guidance: "Start Knot with `cargo run` from the Knot
     project directory, or run the Knot binary."
   - Do NOT proceed further until the user confirms Knot is running.

3. **If Knot IS running**, check rig configuration:
   - Send `GET /config/rig` to `http://localhost:3000/config/rig`.
   - Expected response: `200 OK` with JSON body:
     ```json
     {
       "rig_path": "/absolute/path/to/rig",
       "cli_path": "pi",
       "cli_args": []
     }
     ```
   - This confirms the rig configuration is loaded (defaults or custom).

4. **Ensure rig directory structure exists**:
   - Create `rig/profiles/` if it doesn't exist.
   - These directories are managed on disk — Knot auto-discovers them.

5. **Check for existing profiles**:
   - Send `GET /profiles` to `http://localhost:3000/profiles`.
   - If profiles exist, report the available profile names and skip to
     step 7.

6. **Create default profile** (only when no profiles exist):
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

7. **Verify profile creation**:
   - Send `GET /profiles` to confirm at least one profile exists.
   - Send `GET /profiles/default` to confirm the default profile is
     accessible (only if created in step 6).

8. **Check for existing looms**:
   - Send `GET /looms` to `http://localhost:3000/looms`.
   - If the response is an empty array `[]`, the rig has no looms yet.
     Report: "Rig is initialised but has no looms. Use the `knot-create`
     skill to create looms."
   - If the response contains looms, report the existing loom IDs and
     suggest using the `knot-inspect` skill to examine them.

9. **Report success**: Summarise the rig state including:
   - Knot service status (running)
   - Rig configuration (from `/config/rig`)
   - Profiles available (from `/profiles`)
   - Number of registered looms
   - Next steps (create looms with `knot-create` skill)

---

## API Reference

Before making calls, review the OpenAPI spec at:
`http://localhost:3000/swagger-ui/openapi.json`

### Endpoints Used by This Skill

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Verify Knot is running |
| `/config/rig` | GET | Read rig configuration |
| `/looms` | GET | List registered looms |
| `/profiles` | GET | List agent profiles |
| `/profiles/{name}` | GET | Get a profile by name |

### Expected Response Schemas

**GET /health** → `200` with plain text body `ok`

**GET /config/rig** → `200` with `RigConfigResponse`:
```json
{
  "rig_path": "/absolute/path/to/rig",
  "cli_path": "pi",
  "cli_args": []
}
```

**GET /looms** → `200` with `Array<LoomSummary>`:
```json
[
  {
    "id": {"0": "prd-review-loom"},
    "knot_count": 2
  }
]
```

**GET /profiles** → `200` with `Array<ProfileResponse>`:
```json
[
  {
    "name": "default",
    "provider": "llama-workhorse",
    "model": "qwen3-27b",
    "tools": [],
    "system_prompt": "You are a helpful AI assistant."
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
| `GET /profiles` returns empty array | No profiles exist. Create default profile. |
| `~/.pi/agent/models.json` not found | Use placeholder provider/model. Document in profile body. |

---

## Quick Reference

```bash
# Check if Knot is running
curl http://localhost:3000/health

# View rig configuration
curl http://localhost:3000/config/rig

# List profiles
curl http://localhost:3000/profiles

# Get default profile
curl http://localhost:3000/profiles/default

# List looms
curl http://localhost:3000/looms

# View full API documentation
# Open browser: http://localhost:3000/swagger-ui
```

---

## Cross-Reference

After initialisation, the workflow continues with:

1. **knot-create skill** — create looms, knots, and profiles (file-first)
2. **knot-inspect skill** — inspect rig, loom, and knot state

This skill prepares the rig. The other skills manage the content.
