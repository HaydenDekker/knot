---
name: knot-inspect
description: "Inspect the current state of a Knot rig: list looms, examine loom details, view activity logs, check knot processing status, list agent profiles. Read rig state from `rig/state.json` and activity from `rig/tie-offs/{loom-id}/.loom-log`. USE FOR: inspect rig, check rig status, view looms, list looms, inspect loom, loom status, knot status, check knot, view activity, loom activity, processing status, knot state, rig state, what looms exist, show looms, loom details, list profiles, view profile, check profile. DO NOT USE FOR: creating looms (use knot-create), deleting looms (use knot-create), creating profiles (use knot-create), initialising a rig (use knot-init), triggering processing."
license: MIT
metadata:
  author: Knot Team
  version: "3.0.0"
  compatibility: "Knot 0.18.0+"
---

# Knot Inspect Skill

Inspect the current state of a Knot rig. This skill provides read-only
access to rig configuration, loom details, activity logs, knot
processing status, and agent profiles by reading `rig/state.json`
and loom activity log files.

**State file:** `rig/state.json` (written every 5 seconds by Knot)
**Activity logs:** `rig/tie-offs/{loom-id}/.loom-log` (append-only JSONL)

---

## Core Philosophy

### Read-Only

This skill only reads state. It does not modify, create, or delete any
resources. Use `knot-init` or `knot-create` for write operations.

### File-First

All state is in files. Read `rig/state.json` for current rig state.
Read `.loom-log` files for historical activity. No HTTP calls needed.

### Progressive Disclosure

Start broad (rig overview), then drill down (specific loom, then specific
knot) based on user requests.

---

## Prerequisites

1. Knot must be running and `rig/state.json` must exist.
   If the file does not exist, Knot has not started or the rig is not
   initialised. Use `knot-init` skill.

---

## State File Schema

`rig/state.json` contains the current snapshot of rig state:

```json
{
  "rig_path": "/absolute/path/to/rig",
  "looms": [
    {
      "id": "prd-review-loom",
      "knots": [
        {
          "id": "goals-review",
          "status": "completed",
          "last_strand_path": "project/prds/goals.md",
          "last_tie_off_path": "rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md",
          "last_error": null,
          "last_event_at": "2026-06-10T12:00:03Z"
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

The state file is written atomically every 5 seconds. Staleness is at
most 5 seconds behind reality.

---

## Agent Workflow

### Inspect the Full Rig

When asked to show rig status:

1. **Read state file**: Read `rig/state.json`.
   If the file does not exist, report: "Knot is not running or rig is
   not initialised. Use `knot-init` skill."

2. **Show rig configuration**: Extract `rig_path` from the state file.
   Report the rig path.

3. **List looms**: Extract the `looms` array from state.
   Present a summary table:

   | Loom ID | Knot Count |
   |---------|-----------|
   | `prd-review-loom` | 2 |

4. **List profiles**: Extract the `profiles` array from state.
   Present a summary table:

   | Profile | Provider | Model | Timeout |
   |---------|----------|---------|---------|
   | `fast` | `openai` | `gpt-4o` | `300` |

5. **If no looms**: Report "No looms are registered. Use the
   `knot-create` skill to create looms."

### Inspect a Specific Loom

When asked about a specific loom (by ID):

1. **Read state file**: Read `rig/state.json`.
   Find the loom with matching `id` in the `looms` array.
   - If not found: Report "Loom `{id}` not found. Check `rig/state.json`
     to see available looms."

2. **Show loom configuration**:
   - Loom ID
   - List of knots with their status and last processed strand

3. **Get activity log**: Read `rig/tie-offs/{loom-id}/.loom-log`.
   - If the file does not exist: Report "No activity log found for this
     loom."
   - Present the activity entries in chronological order:
     - `LoomStarted` events
     - `KnotRegistered` events
     - `KnotProcessing` events (with strand path)
     - `KnotCompleted` events (with strand and tie-off paths)
     - `KnotFailed` events (with error message)
     - `StrandProcessed` events (with error if any)

### Inspect a Specific Knot

When asked about a specific knot within a loom:

1. **Read state file**: Read `rig/state.json`.
   Find the loom, then find the knot with matching `id` in the loom's
   `knots` array.
   - If not found: Report "Knot `{knot_name}` not found in loom
     `{loom_id}`."

2. **Show knot state**:
   - Knot ID and Loom ID
   - Current processing status (`idle`, `processing`, `completed`, `failed`)
   - Last processed strand path
   - Last tie-off output path (if produced)
   - Error message (if failed)
   - Last event timestamp

### Inspect All Knot States

When asked to show status of all knots across all looms:

1. Read `rig/state.json`.
2. Iterate over all looms and their knots.
3. Present a consolidated table:

   | Loom | Knot | Status | Last Strand | Error |
   |------|------|--------|-------------|-------|
   | `prd-review-loom` | `goals-review` | `completed` | `goals.md` | — |
   | `prd-review-loom` | `non-goals-review` | `failed` | `non-goals.md` | timeout |

### Inspect Profiles

When asked to list or view agent profiles:

1. **List all profiles**: Read `rig/state.json` and extract the
   `profiles` array.
   Present a summary table with: Name, Provider, Model, Timeout
   (show "default" for null/missing values).

2. **View a specific profile**: Find the profile by name in state.
   - If not found: Report "Profile `{name}` not found. Check
     `rig/state.json` to see available profiles."
   - Show: name, provider, model, timeout.
   - The state file includes `timeout` (in seconds). A missing or
     null value means the runner default of 300 seconds (5 minutes).
   - The state file does not include `profile_prompt`. If the user
     asks about it, read the file directly from
     `rig/profiles/{name}.md` and check the YAML frontmatter.

---

## Activity Log Format

Each loom has an append-only JSONL activity log at
`rig/tie-offs/{loom-id}/.loom-log`. Each line is a JSON object
representing one event.

### Event Types

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

### Example Activity Entry

```json
{"KnotCompleted":{"loom_id":"prd-review-loom","knot_id":"goals-review","strand_path":"project/prds/goals.md","tie_off_path":"rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md","timestamp":"2026-06-10T12:00:03Z"}}
```

---

## Processing Status Values

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `rig/state.json` does not exist | Knot is not running or rig not initialised. Suggest `knot-init` skill. |
| `rig/state.json` is invalid JSON | State file may be corrupt. Report to user. |
| Loom `{id}` not in state | Loom not found. May not have been discovered yet. Check `rig/` for directories ending in `-loom`. |
| Knot `{name}` not in loom | Knot not found. Check loom directory for `{name}.md`. |
| Activity log file missing | Loom may have no events yet. No error. |

---

## Quick Reference

```bash
# View current rig state
cat rig/state.json
# or with pretty printing:
python3 -m json.tool rig/state.json

# View loom activity log
cat rig/tie-offs/prd-review-loom/.loom-log

# View a specific profile
cat rig/profiles/fast.md

# Watch state file for updates (wait for loom discovery)
watch -n 5 'cat rig/state.json | python3 -m json.tool'
```

---

## Cross-Reference

Related skills:

1. **knot-init skill** — initialise the rig
2. **knot-create skill** — create, modify, or delete looms, knots, and profiles

This skill provides visibility into rig state. Use knot-create for
changes.
