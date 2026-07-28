---
name: knot-create
description: "Create looms, knots, and profiles by writing .md files directly. Knot auto-discovers looms (directories ending in `-loom`) and parses knot definition files (`.md` files inside loom directories). Profiles live in `rig/profiles/`. Read `rig/state.json` to verify state after file changes. USE FOR: create loom, add loom, new loom, delete loom, remove loom, modify loom, update loom, create knot, add knot, configure knot, loom CRUD, knot CRUD, loom management, knot management, create profile, agent profile, profile CRUD. DO NOT USE FOR: initialising a rig (use knot-init), inspecting state (use knot-inspect), triggering processing, running agent sessions."
license: MIT
metadata:
  author: Knot Team
  version: "5.4.0"
  compatibility: "Knot 0.26.0+"
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
 │           ├── tie-off-{knot-name}.md  ← append-only log
 │           └── {event-type}/           ← tie-off events
 │                 └── {event}.md        ← static event strand
 └── {name}-loom/            ← loom directory (must end in `-loom`)
      ├── {knot-name}.md     ← knot definition files
      └── ...
```

- A **rig** is the top-level container for all looms and profiles.
- A **loom** is a directory inside `rig/` whose name ends in `-loom`
  (e.g. `prd-review-loom`). Knot discovers these automatically.
- A **knot** is a `.md` file with YAML frontmatter inside a loom
  directory. It references a shared **agent profile** via
  `agent-profile-ref`. The markdown body (after the closing `---`)
  contains the knot's task-specific instructions. Each knot has a
  single input direction declared as `strand-dir` — either a filesystem
  path or an `event:` URI for event consumer knots.
- An **agent profile** is a `.md` file with YAML frontmatter stored in
  `rig/profiles/{name}.md`. The frontmatter holds structural metadata
  (name, provider, model, tools, timeout) and the markdown body contains
  the agent's system prompt (persona instructions). Multiple knots can
  reference the same profile.
- **Event dispatch** subdirectories are created automatically by Knot
  inside `rig/tie-offs/{loom-id}/{EventId}/` when a consumer knot uses
  an `event:` URI in its `strand-dir`. These carry dispatched event
  files that trigger consumer knots.

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
   - System prompt: The agent's persona instructions (goes in the
     markdown body after the closing `---`)
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
   ---

   You are a fast reviewer. Keep responses concise and direct.
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
   ---

   You are a code generation agent. Take your time to be thorough.
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
   values. Edit frontmatter for structural metadata (name, provider,
   model, tools, timeout) and the markdown body for the system prompt.

3. **Verify the change**: Read `rig/state.json` and confirm the profile
   entry is present. Note: the system prompt (body) is not in the state
   file — verify by re-reading the profile file. `timeout` is included
   in state.

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

5. **Determine event subscriptions** for each knot. Ask the user:
   "Does this knot consume events from another knot? (e.g.
   `event:quality-reviewer:ReviewCompleted`) — or from an entire loom?
   (e.g. `event:planning-loom:PlanCreated`) — or does it read from a
   normal filesystem directory?"
   If the knot consumes events, set `strand-dir` to the `event:` URI
   and optionally add `event-description`. Knot will create and watch
   the dispatch directory automatically. For loom-level subscriptions,
   the target must end in `-loom`.

6. **Write knot definition files** inside the loom directory.
   For a single knot named `goals-review`:
   ```markdown
   ---
   name: goals-review
   agent-profile-ref: fast
   strand-dir: "project/prds"
   ---

   Review the goals section for clarity and measurability.
   ```
   Write this to `rig/prd-review-loom/goals-review.md`.

7. **Verify registration**: Read `rig/state.json` (wait up to 5 seconds
   for the state writer to flush) and confirm the loom and its knots
   appear in the `looms` array.

7. **Report success**: "Loom `prd-review-loom` created with 1 knot."

### Add a Knot to an Existing Loom

When asked to add a knot to an existing loom:

1. **Verify the loom exists**: Read `rig/state.json` and find the loom
   in the `looms` array.

2. **Verify the profile exists**: Read `rig/state.json` and check the
   `profiles` array for the knot's `agent_profile_ref`.

3. **Determine event subscription**. Ask the user:
   "Does this knot consume events from another knot? Or from an entire
   loom?"
   If so, set `strand-dir` to the `event:` URI and optionally add
   `event-description`. For loom-level subscriptions, the target must
   end in `-loom`. Knot will create and watch the dispatch directory
   automatically.

4. **Write the knot file** as `{knot-name}.md` inside the loom
   directory (e.g. `rig/prd-review-loom/non-goals-review.md`):
   ```markdown
   ---
   name: non-goals-review
   agent-profile-ref: fast
   strand-dir: "project/prds"
   ---

   Review the non-goals section.
   ```

5. **Verify**: Read `rig/state.json` (wait up to 5 seconds) and confirm
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
---

Review the goals section of this PRD. Check that:
- Each goal is specific and measurable
- Goals align with the problem statement
```

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | **Yes** | Unique knot identifier (becomes the `KnotId`) |
| `agent-profile-ref` | **Yes** | Name of the agent profile to use (must exist in `rig/profiles/{name}.md`) |
| `strand-dir` | **Yes** | Input source — either a filesystem path (e.g. `"project/prds"`) or an `event:` URI. Event URIs support knot-level (`"event:quality-reviewer:ReviewCompleted"`) and loom-level (`"event:planning-loom:PlanCreated"`) subscriptions. Paths are resolved relative to the project root. |
| `event-description` | No | Semantic description of the event, injected into the producer's prompt. Only meaningful when `strand-dir` is an `event:` URI. |
| `git-versioned` | No | Whether to git-commit after each successful knot run. Defaults to `true`. Set to `false` to opt out. |

### Markdown Body

The text after the closing `---` is the knot's task-specific
instructions. This content is appended to the profile's system prompt
at processing time to form the full prompt sent to the agent.

The body must not be empty or contain only whitespace — the parser
will reject such files with a `KnotParseWarning`.

### Directory Resolution

- `strand-dir` is **relative to the project root**
  (the directory containing the `rig/` folder).
- Absolute paths are used as-is.
- Tie-off paths are statically derived:
  `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`

### Event Routing

Knots that consume events from other knots use an `event:` URI in their
`strand-dir` field. This is the single input-direction primitive —
each knot has exactly **one** `strand-dir`, whether it reads from the
filesystem or from event dispatch.

**Event URI format:**

```
event:<producer-target>:<EventId>
```

Three colon-separated parts. No escaping needed — targets are
kebab-case slugs (knot IDs or loom IDs), event IDs are PascalCase
identifiers.

**Two subscription levels:**

- **Knot-level** (existing): `event:<knot-name>:<EventId>` — subscribe
to events from a *specific knot*. Example:
  `event:quality-reviewer:ReviewCompleted`
- **Loom-level** (new in 0.26.0): `event:<loom-name>:<EventId>` —
  subscribe to events from *any knot* within a loom. The target must
  end in `-loom`. Example:
  `event:planning-loom:PlanCreated`

Loom-level subscriptions are useful when multiple knots in a loom can
emit the same event type — the consumer subscribes once instead of
once-per-knot. Every knot in the subscribed-to loom receives event
injection instructions in its prompt.

**Consumer knot — knot-level (declares subscription via `strand-dir`):**

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "event:quality-reviewer:ReviewCompleted"
event-description: >
  Emitted when a quality review is complete and findings are
  ready for planning.
---

Create a refactor plan when a quality review is complete.
```

The `event-description` field provides the semantic contract injected
into the producer's prompt. When absent, a generic message is injected.

**Consumer knot — loom-level (subscribe to any knot in a loom):**

```markdown
---
name: change-tracker
agent-profile-ref: fast
strand-dir: "event:planning-loom:PlanCreated"
event-description: >
  Emitted when any knot in the planning loom creates a new plan.
---

Track all plan creation events.
```

With a loom-level subscription, **every knot** inside `planning-loom`
receives event injection instructions. Events emitted by *any* knot
in that loom are dispatched to the consumer. The `-loom` suffix on
the target is the heuristic that distinguishes loom-level from
knot-level subscriptions.

**Producer knot (no declaration needed — Knot injects instructions):**

Before the producer knot runs, Knot scans all other knots' `strand-dir`
values for `event:` URIs that reference this knot as the producer, and
injects event instructions at the **beginning** of its prompt. The
injected block includes a `## Agent Events` heading, event descriptions,
and instructions to always emit an event block:

```
## Agent Events

Other knots are listening for events you may emit. If an event occurs
during your work, include an explicit event block in your tie-off using
the format shown.

Events you may emit:
- `ReviewCompleted` — Emitted when a quality review is complete and
  findings are ready for planning.

If an event occurred, emit in your tie-off:
```
event: ReviewCompleted
description: <short summary of what happened>
<additional fields as relevant>
```

If no events occurred, emit:
```
event: None
```
```

The producer writes the event block in its tie-off. Knot parses it,
matches to consumer `event:` URIs, and creates event files in each
consumer's dispatch directory (`rig/tie-offs/{loom-id}/{event-id}/`).

**How it works:**

1. Consumer sets `strand-dir` to an `event:` URI
   (`event:<producer-target>:<EventId>`), where `<producer-target>`
   is either a knot name (knot-level) or a loom name ending in
   `-loom` (loom-level).
2. Consumer optionally provides `event-description` for the semantic
   contract injected into the producer's prompt.
3. Knot creates and watches the dispatch directory:
   `rig/tie-offs/{loom-id}/{event-id}/`.
4. Before a producer knot runs, Knot injects event instructions into
   its prompt (grouped by `event-id`, deduplicated across consumers):
   - For **knot-level** subscriptions: instructions are injected only
     into the named producer knot's prompt.
   - For **loom-level** subscriptions: instructions are injected into
     **every knot** within the subscribed-to loom.
5. Producer emits a structured event block in its tie-off
   (`event: EventId` or `event: None`).
6. Knot parses the tie-off and matches events:
   - **Knot-level**: `target == producer_knot_id` → match.
   - **Loom-level**: `target == producer_loom_id` (target ends in
     `-loom`) → match. If the target matches both a knot name and
     a loom name, loom-level takes precedence.
7. Matching events create files in each consumer's dispatch directory.

**Layout:**

```
rig/tie-offs/<loom-id>/
├── tie-off-<knot-name>.md      ← append-only log (always present)
├── tie-off-<another-knot>.md   ← another knot's tie-off (flat)
└── <EventId>/                ← dispatch subdirectory (created by Knot)
      └── <event-file>.md      ← dispatched event strand for consumers
```

Multiple consumers can subscribe to the same producer event — each gets
its own dispatch directory in its loom's tie-off directory.

**Loom-level vs knot-level — dispatch behaviour:**

Both subscription levels write event files to the *same* dispatch
directory (`rig/tie-offs/{consumer-loom-id}/{event-id}/`). The only
difference is *which producers match*:

- **Knot-level**: only the named knot's events match.
- **Loom-level**: events from *any* knot in the named loom match.

When a consumer has both a knot-level and a loom-level subscription
for the same `EventId`, Knot deduplicates by event ID — the consumer
receives the event once regardless of how many subscriptions matched.

### Example Project Layout

```
project_root/              ← strand-dir resolves from here
├── project/prds/          ← strand-dir: "project/prds"
└── rig/                   ← rig directory
    ├── profiles/          ← shared agent profiles
    │   ├── fast.md
    │   └── coder.md
    ├── tie-offs/          ← tie-off directory
    │   ├── prd-review-loom/
    │   │   ├── .loom-log
    │   │   └── tie-off-prd-goals-review.md
    │   └── planning-loom/
    │       ├── .loom-log
    │       ├── tie-off-refactor-planner.md
    │       └── ReviewCompleted/  ← dispatch dir (auto-created by Knot)
    │           └── event-2026-07-10T12-00-00.md
    ├── prd-review-loom/   ← loom with normal knot
    │   └── prd-goals-review.md  ← strand-dir: "project/prds"
    └── planning-loom/     ← loom with event consumer knot
        └── refactor-planner.md  ← strand-dir: "event:quality-reviewer:ReviewCompleted"
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
---

You are a fast reviewer. Keep responses concise and direct.
```

### Profile Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | **Yes** | Profile identifier (becomes the filename stem) |
| `provider` | **Yes** | LLM provider (e.g. `openai`, `anthropic`) |
| `model` | **Yes** | Model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`) |
| `tools` | No | List of pi tool names (e.g. `read`, `write`, `edit`, `bash`). Defaults to empty. Pi's built-in tools: `read`, `bash`, `edit`, `write`, `grep`, `find`, `ls`. |
| `timeout` | No | Session timeout in seconds. If omitted, the runner's default of 300 seconds (5 minutes) is used. When a session exceeds its timeout, a `TimeoutExceeded` event is recorded in the rig-log and the tie-off file is preserved unchanged. |

### Profile Markdown Body

The text after the closing `---` is the agent's system prompt
(persona instructions). This is the primary content of the profile.

The body must not be empty or contain only whitespace — the parser
will reject such files.

### How Profiles Are Used at Processing Time

When a strand event triggers a knot:

1. The knot's `agent-profile-ref` is used to load the profile from
   `rig/profiles/{name}.md` (read fresh from disk each time).
2. The profile provides: `provider`, `model`, `tools`.
3. The profile's markdown body is merged with the knot's markdown
   body to form the full system prompt:
   ```
   {profile body}

   {knot body}
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
> `timeout` for profiles but not `tools` or the system prompt (body).
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
---

You are a fast reviewer.
EOF

# Create a loom with a normal knot (filesystem strand-dir)
mkdir -p rig/prd-review-loom
cat > rig/prd-review-loom/goals-review.md << 'EOF'
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section.
EOF

# Create an event consumer knot — knot-level (event: URI strand-dir)
cat > rig/planning-loom/refactor-planner.md << 'EOF'
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "event:quality-reviewer:ReviewCompleted"
event-description: >
  Emitted when a quality review is complete and findings
  are ready for planning.
---

Create a refactor plan when a quality review is complete.
EOF

# Create an event consumer knot — loom-level (subscribe to entire loom)
cat > rig/planning-loom/change-tracker.md << 'EOF'
---
name: change-tracker
agent-profile-ref: fast
strand-dir: "event:prd-review-loom:ReviewCompleted"
event-description: >
  Emitted when any knot in prd-review-loom completes a review.
---

Track all review completions from the PRD review loom.
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

**Before using this skill:** Read the `knot-abstractions` skill for the
layered architecture overview. Understanding the rig/profile/skill/application
boundary helps ensure you design knots with proper separation of concerns.

Related skills:

1. **knot-abstractions skill** — foundational architecture overview
2. **knot-init skill** — initialise the rig (prerequisite for this skill)
3. **knot-inspect skill** — inspect loom activity and knot processing state
4. **knot-design skill** — design principles for idempotent, loop-safe knots

This skill (`knot-create`) manages the full loom, knot, and profile
lifecycle through direct file operations. Use knot-inspect for
monitoring and debugging.
