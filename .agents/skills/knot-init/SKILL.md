---
name: knot-init
description: "Initialise a Knot rig in the current directory. Detects if a rig exists, verifies Knot is running by checking `tie-offs/rig/state.json`, and creates the rig directory structure. If no profiles exist, seeds a `default` alias in `rig/models.yml` from ~/.pi/agent/models.json and creates a default profile that uses `model-ref: default`. Verifies setup by reading `tie-offs/rig/state.json`. USE FOR: init knot, knot init, setup knot, configure knot rig, initialise knot, knot configuration, rig init, rig setup. DO NOT USE FOR: starting or stopping the Knot service (use knot-start), creating looms, creating knots, inspecting loom state, modifying existing looms."
license: MIT
metadata:
  author: Knot Team
  version: "4.10.0"
  compatibility: "Knot 0.41.1+"
---

# Knot Init Skill

Initialise a Knot rig in the current working directory. This skill detects
whether a rig already exists, verifies that Knot is running by checking
for `tie-offs/<rig>/state.json`, creates the rig directory structure, and
sets up a default agent profile if none exist.

**State file:** `tie-offs/<rig>/state.json` (written when the state
changes — Knot 0.41.0+; the baseline is written immediately at startup,
so a fresh `updated_at` is proof of a recent start).
For the default rig this is `tie-offs/rig/state.json`; for a named
rig (e.g. `dev-rig`) it is `tie-offs/dev-rig/state.json`.

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
agent configuration at `~/.pi/agent/models.json` and seeds a
`default` alias in `rig/models.yml`. The default profile then
references that alias via `model-ref: default` — the registry is
resolved fresh at processing time, so swapping the model behind the
alias is a single edit to `rig/models.yml` (no profile edits, no
restart).

---

## Prerequisites

1. Knot must be compiled and available (e.g. via `cargo run` or
   installed binary)

---

## Agent Workflow

When asked to initialise a Knot rig:

1. **Check if Knot is running**: Check if `tie-offs/<rig>/state.json`
   exists (default rig: `tie-offs/rig/state.json`).
   - If the file exists and is valid JSON, Knot is running and the rig
     is initialised.
   - If the file does not exist, Knot may not be running or the rig has
     not been initialised yet.

2. **If Knot is NOT running**:
   - Check if `tie-offs/<rig>/state.json` exists:
     - If it exists but is older than 10 seconds (check `updated_at`),
       Knot may be slow to start. Wait and re-check.
     - If it does not exist, report that Knot is not reachable.
   - Start Knot with the `knot-start` skill (it runs the service in the
     background and appends its output to
     `tie-offs/<rig>/knot-service.log`, which this skill then reads to
     confirm a clean boot). Do not launch it with a bare `cargo run`
     from the agent session — the call blocks and the output is lost.
   - Do NOT proceed further until the service is confirmed running.

3. **If Knot IS running**, check rig state:
   - Read `tie-offs/<rig>/state.json`.
   - Extract `rig_path` to confirm the rig configuration is loaded
     (defaults or custom).

4. **Ensure rig directory structure exists**:
   - Create `rig/profiles/` if it doesn't exist.
   - These directories are managed on disk — Knot auto-discovers them.

4a. **Install Knot skills globally** (idempotent):
    - Production layout: sub-skills go to `~/.agents/skills-library/`
      (not auto-discovered; read on demand via the master's routing
      table) and the master router goes to `~/.agents/skills/knot/`
      (auto-discovered). Check both exist. Create them if missing.
    - Copy the *contents* of each Knot sub-skill directory into
      `~/.agents/skills-library/<skill>/` (the trailing `/.` is
      required — copying onto an existing directory would nest it):
      ```bash
      for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
                    knot-manage knot-design knot-analyst knot-update
                    knot-abstractions; do
        mkdir -p ~/.agents/skills-library/$skill
        cp -r .agents/skills/$skill/. ~/.agents/skills-library/$skill/
      done
      ```
    - Copy the master router into the auto-discovered location:
      ```bash
      mkdir -p ~/.agents/skills/knot
      cp -r .agents/skills/knot/. ~/.agents/skills/knot/
      ```
    - Also copy any non-SKILL.md files in skill directories
      (e.g. `.agents/skills/knot-init/knot-glossary.md` →
      `~/.agents/skills-library/knot-init/knot-glossary.md`).
    - **Verify every copy succeeded** — `cp` can silently fail on
      permissions or stale handles. After copying, diff each skill:
      ```bash
      for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
                    knot-manage knot-design knot-analyst knot-update
                    knot-abstractions; do
        diff .agents/skills/$skill/SKILL.md \
             ~/.agents/skills-library/$skill/SKILL.md > /dev/null 2>&1 && \
          echo "$skill: OK" || echo "$skill: FAILED"
      done
      diff .agents/skills/knot/SKILL.md \
           ~/.agents/skills/knot/SKILL.md > /dev/null 2>&1 && \
        echo "knot: OK" || echo "knot: FAILED"
      ```
    - If any skill reports FAILED, retry the copy for that skill.
    - If the glossary exists in the project skill directory, verify
      it too:
      ```bash
      if [ -f .agents/skills/knot-init/knot-glossary.md ]; then
        cp .agents/skills/knot-init/knot-glossary.md \
           ~/.agents/skills-library/knot-init/knot-glossary.md
        diff .agents/skills/knot-init/knot-glossary.md \
             ~/.agents/skills-library/knot-init/knot-glossary.md > /dev/null 2>&1 && \
          echo "glossary: OK" || echo "glossary: FAILED"
      fi
      ```

4c. **Ensure the rig repository is healthy** (informational, idempotent):
   - Knot initialises `rig/.git` on startup (idempotent). The rig has
     its **own git repository** and is committed **manually by the user**
     — Knot never commits the rig git.
   - When the project root is inside a git repo, Knot appends a marked
     root-anchored `/rig/` line to the project's `.gitignore` so the rig can
     never be swept into project commits (gitlink or tracked leftovers).
     The leading slash anchors the pattern to the repo root, so nested
     directories such as `tie-offs/rig/` stay versioned (Knot 0.41.1+;
     a bare `rig/` line from an older binary is rewritten in place on
     the next startup).
   - **Pre-existing projects** (where `rig/` files were already tracked
     by the parent repo before the 0.31.0 migration): Knot logs a
     warning. The user must run the one-time untrack step once:
     ```bash
     git rm -r --cached rig/
     git commit -m "Untrack rig/ — now versioned in its own repository"
     ```
     After this, the `.gitignore` entry holds and Knot's project
     commits never touch `rig/` again. The Knot binary must not run
     `git rm` itself — untracking is a project-history decision.
   - Verify with `git status` in the project root: `rig/` should be
     untracked, and runtime data should appear under `tie-offs/<rig>/`.

4b. **Ensure Knot section in AGENTS.md** (idempotent):
    - Read `AGENTS.md` from the project root if it exists.
    - Check if it already contains Knot information (search for
      `knot-init`, `knot-create`, "## Agent Skills" with Knot skills
      listed, or `## Knot Terminology`). If present, skip this step
      entirely.
    - If `AGENTS.md` does NOT exist, create it with a full document
      containing a project header, build/run instructions, and the
      Knot sections below.
    - If `AGENTS.md` exists but has no Knot information, append the
      following sections after any existing content:
      ```markdown

      ## Rig

      This project uses **Knot** for agent orchestration. Knot runs as a
      local service and manages AI agent workflows through looms and
      knots defined in `rig/`.

      ### Running

      Start the Knot service with the `knot-start` skill — it runs the
      service in the background and **appends** its output to the
      service log (Knot's structured logs are cleared at every startup,
      so this file is the only record across runs):

      ```bash
      mkdir -p tie-offs/rig
      nohup knot >> tie-offs/rig/knot-service.log 2>&1 &
      echo $! > tie-offs/rig/knot-service.pid
      ```

      ### Knot Terminology

      Basic Knot terms used throughout the rig:

      - **rig** — the top-level container holding looms and profiles (reusable source; its own git repository)
      - **runtime tree** — `tie-offs/<rig>/`: the rig's project-side runtime data (tie-offs, logs, event queue, state), committed with the project
      - **loom** — a domain work area (a directory ending in `-loom`) grouping related knots
      - **knot** — a configured task/agent workflow that processes input strands
      - **strand** — a file in a knot's strand-dir that triggers the knot to process it
      - **tie-off** — a knot's final output document, stored under `tie-offs/<rig>/`
      - **event** — a message a producer knot emits for consumer knots to process

      Knot terminology is encouraged inside rig files. Keep this
      terminology out of skill documents and project-space documents.

      ### Agent Skills

      This project uses the following Knot skills:

      - **knot-init** — Initialise the rig
      - **knot-start** — Start/stop/restart the service, capture the service log
      - **knot-create** — Create, modify, delete looms, knots, and profiles
      - **knot-dispatch** — Trigger knots into action (strand files, events)
      - **knot-inspect** — Inspect rig state (looms, knots, activity)
      - **knot-manage** — Review rig work via git and tie-offs
      - **knot-design** — Design looms and knots (idempotency, naming, loops)
      - **knot-analyst** — Analyse rig productivity and project progress
      - **knot-update** — Migrate project documents between Knot versions
      ```
    - Read the Knot glossary from this skill's directory at
      `knot-glossary.md`. It covers the full set of Knot domain terms
      and must be read before working on any Knot feature. The six
      basic terms above are appended to AGENTS.md so they are always
      available during any agent session; the complete glossary stays
      in the skill for deeper reference.
    - If the project also has a `project/domain-glossary.md` (a
      project-specific glossary separate from the Knot glossary),
      include a domain glossary reference at the end of the
      appended content:
      ```markdown

      ### Domain Glossary

      Project-specific domain terms are defined in
      [project/domain-glossary.md](project/domain-glossary.md). Read
      it before starting work on any feature.
      ```

5. **Agent adapter configuration** (`rig/.workspace-agent-config.yaml`):
   - Knot auto-creates this file on first startup if it doesn't exist
     (default `agent-adapter: pi-json` since 0.40.0).
   - It controls how Knot invokes the Pi agent. Default content:
     ```yaml
     # Rig-level agent configuration.
     #
     # agent-adapter: which adapter to use for Pi invocations.
     #   pi-stdio — plain text stdout
     #   pi-json  — JSON-L stream with session ID + token usage capture
     #              (default; also enables inactivity restart with
     #              blocked-call identification)
     #   pi-rpc   — JSON-over-stdin/stdout RPC (`pi --mode rpc`);
     #              keeps stdin open so Knot can steer the session
     #              mid-run (context wrap-up; 0.42.0+)
     #
     # inactivity-timeout-seconds: kill a session that produces no
     # output for this window and restart it with a blocking-call
     # note. Default 300; 0 disables the watchdog. (Knot 0.40.0+)
     #
     agent-adapter: pi-json
     ```
   - `pi-json` — **default** (Knot 0.40.0+). JSON-L output, captures
     session ID and token usage. Required for session-resume restarts
     and invocation visibility; with the inactivity watchdog (0.40.0+)
     it also names the blocked call in the restart note.
   - `pi-stdio` — plain text output. No metadata capture (no session
     ID, no token usage); inactivity detection still works, but
     restarts after a stall are fresh sessions (no `--session-id`).
   - `pi-rpc` (Knot 0.42.0+) — pi's `--mode rpc` JSONL protocol over
     stdin/stdout. Like `pi-json` it captures session ID and token
     usage, but it **keeps stdin open** across the run so Knot can
     steer the session mid-run. Combined with a per-alias
     `ctx-wrap-up-limit` in `rig/models.yml`, it sends a one-shot
     wrap-up steer when the context crosses the limit (see
     `ContextWrapUpSteered`). `pi-json` remains the default; `pi-rpc`
     is opt-in.
   - `inactivity-timeout-seconds` (Knot 0.40.0+) — default 300; set
     to `0` to disable the watchdog. Loaded at startup — restart Knot
     after editing.
   - To switch: edit `rig/.workspace-agent-config.yaml`, change
     `agent-adapter` to the other value, restart Knot. (`pi-rpc` also
     needs a `ctx-wrap-up-limit` on the model alias to steer — see
     `knot-create`.)

5a. **Enable pi compaction for rig sessions** (`.pi/settings.json`;
   idempotent, create-if-absent only):
   - Check whether `.pi/settings.json` exists at the project root.
     If it exists, **leave it untouched** — never overwrite an
     existing settings file (it may carry operator choices beyond
     compaction).
   - If it does not exist, create the `.pi/` directory and write:
     ```json
     { "compaction": { "enabled": true } }
     ```
   - Read the file back and verify it parses as JSON and contains
     the compaction block.
   - Why: pi resolves the project-level `.pi/settings.json` over the
     global settings, and knot spawns pi inheriting the rig project's
     CWD — so compaction applies only to rig sessions in this
     directory, never to interactive pi elsewhere. With compaction on,
     pi recovers from a context overflow in-process
     (compact-and-continue) instead of ending the turn in error; knot
     records each compaction as a `ContextCompacted` service-log event
     and fails the strand immediately (`ContextLimitReached`) when
     the context still cannot fit after compaction (Knot 0.38.0+).

6. **Check for existing profiles**:
   - Read `tie-offs/<rig>/state.json` and extract the `profiles` array.
   - If profiles exist, report the available profile names and skip to
     step 8.

7. **Create default profile + seed the model registry** (only when no
   profiles exist):
   - Read available models from `~/.pi/agent/models.json` to determine
     a provider and model for the `default` alias.
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
   - **Seed the `default` alias in `rig/models.yml`** (Knot's
     `run_startup` auto-creates the file as a commented template when
     missing — never overwrite user content):
     - If `rig/models.yml` already defines a `default` alias, leave
       the file untouched (idempotent).
     - Otherwise, add the alias — create the `models:` section if the
       file has none, and keep any existing aliases:
       ```yaml
       models:
         default:
           provider: <provider-name>
           model: <model-id>
       ```
     - The seeded alias may optionally carry a `thinking-level`
       (Knot 0.36.0+): a default reasoning effort (`off | minimal |
       low | medium | high | xhigh`) for every profile that resolves
       the alias; a profile's own `thinking-level` overrides it. The
       auto-created commented template documents the key. The default
       seed omits it — pi's settings default applies (omission is not
       `off`).
   - Write the default profile to `rig/profiles/default.md`. The
     prompt lives in the **body** (not frontmatter):
     ```markdown
     ---
     name: default
     model-ref: default
     ---

     You are a helpful AI assistant. Follow the instructions
     provided in each task.

     # Default Profile

     Auto-generated default profile. The `default` alias in
     rig/models.yml is seeded from ~/.pi/agent/models.json.

     To change the model, edit the `default` alias in rig/models.yml
     — it is resolved fresh on every run (no restart).
     To add more profiles, create additional .md files in this
     directory (e.g. fast.md, reviewer.md).
     ```
   - If `~/.pi/agent/models.json` does not exist or cannot be read,
     seed a **placeholder** `default` alias and document it in the
     body:
     ```yaml
     models:
       default:
         provider: openai
         model: gpt-4o
     ```
     ```markdown
     ---
     name: default
     model-ref: default
     ---

     You are a helpful AI assistant.

     # Default Profile

     Auto-generated default profile.

     WARNING: Could not read ~/.pi/agent/models.json.
     The `default` alias in rig/models.yml uses placeholder values
     — edit it to configure a real provider and model.
     ```

8. **Verify profile creation**:
   - Read `tie-offs/<rig>/state.json` (wait up to 5 seconds for the
     state writer to flush) and confirm at least one profile exists in
     the `profiles` array.
   - If created in step 7, confirm `default` appears in the list with
     `model-ref: "default"` and a **resolved** (non-null)
     `provider`/`model` — nulls mean the alias is unresolvable (check
     `rig/models.yml`).

9. **Check for existing looms**:
   - Read `tie-offs/<rig>/state.json` and check the `looms` array.
   - If the array is empty `[]`, the rig has no looms yet. Report:
     "Rig is initialised but has no looms. Use the `knot-create` skill
     to create looms."
   - If the array contains looms, report the existing loom IDs and
     suggest using the `knot-inspect` skill to examine them.

10. **Report success**: Summarise the rig state including:
   - Knot service status (running)
   - Rig path (from `tie-offs/<rig>/state.json`)
   - Profiles available (from `tie-offs/<rig>/state.json`)
   - Number of registered looms
   - Next steps (create looms with `knot-create` skill)

---

## State File Schema

`tie-offs/<rig>/state.json` contains the current snapshot of rig state:

```json
{
  "rig_path": "/absolute/path/to/rig",
  "looms": [],
  "profiles": [
    {
      "name": "default",
      "model-ref": "default",
      "provider": "llama-workhorse",
      "model": "qwen3-27b"
    }
  ],
  "updated_at": "2026-06-18T12:00:00Z"
}
```

Profile entries carry `model-ref` (the alias, `null` for direct-spec
profiles) plus the **resolved** `provider`/`model` from
`rig/models.yml`. `null` provider/model means the alias is
unresolvable — the profile's knots will fail with `ModelRefNotFound`
until `rig/models.yml` defines the alias. The optional
`thinking-level` key (Knot 0.36.0+) shows the effective reasoning
effort — the profile's own value, else the alias default; the key is
absent when neither sets one (pi's settings default applies — not
`off`).

The `updated_at` field is an ISO 8601 UTC timestamp. Use it to
determine if the state file is stale (older than ~10 seconds means
Knot may not be writing state).

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `tie-offs/<rig>/state.json` does not exist | Knot is not running or rig not initialised. Start it with the `knot-start` skill. |
| `tie-offs/<rig>/state.json` is invalid JSON | State file may be corrupt or partially written. Wait a moment and re-read. |
| `tie-offs/<rig>/state.json` `updated_at` is stale | Knot may have crashed. Read `tie-offs/<rig>/knot-service.log` (see `knot-start`), then restart. |
| `tie-offs/<rig>/state.json` `rig_path` is empty | Rig config may be missing. Report to user. |
| `tie-offs/<rig>/state.json` `profiles` is empty | No profiles exist. Create default profile. |
| `~/.pi/agent/models.json` not found | Seed the `default` alias in `rig/models.yml` with placeholder provider/model. Document in profile body. |
| Profile shows `null` provider/model in state | The profile's alias is unresolvable — `rig/models.yml` is missing, empty, or malformed, or the alias is undefined. Check `rig/models.yml`. |
| `rig/` still shows as tracked in project git (pre-0.31.0 project) | One-time `git rm -r --cached rig/` + commit (see step 4c). |

---

## Quick Reference

```bash
# Start Knot (use the knot-start skill — it backgrounds the service and
# appends tie-offs/rig/knot-service.log)

# Check if Knot is running (state file exists and is fresh)
cat tie-offs/rig/state.json | python3 -m json.tool

# Check when state was last updated
cat tie-offs/rig/state.json | python3 -c "import sys,json; print(json.load(sys.stdin)['updated_at'])"

# View profiles (name, alias, resolved provider, resolved model)
cat tie-offs/rig/state.json | python3 -c "import sys,json; [print(p['name'], p.get('model-ref'), p['provider'], p['model']) for p in json.load(sys.stdin)['profiles']]"

# View looms
cat tie-offs/rig/state.json | python3 -c "import sys,json; [print(l['id'], len(l['knots']), 'knots') for l in json.load(sys.stdin)['looms']]"

# Rig repository (its own git repo — committed manually by the user)
git -C rig status

# Confirm the project git ignores the rig
git check-ignore -v rig/
```

---

## Rig Repository

Since Knot 0.31.0, the rig is versioned in **its own git repository**
(`rig/.git`), separate from the project:

- **Knot initialises `rig/.git` automatically** on startup (idempotent).
  It writes no `.gitignore` inside the rig — the rig tracks exactly its
  source (looms, knots, profiles, config); it holds no runtime data.
- **The user commits the rig git manually** (`git -C rig add -A && git
  -C rig commit -m ...`). Knot never commits the rig.
- **Parent exclusion:** when the project root is inside a git repo,
  Knot appends a marked root-anchored `/rig/` line to the project's
  `.gitignore` so `git add -A` at the project level can never stage the
  rig (as a gitlink or as tracked leftovers). The anchor (leading slash)
  keeps the pattern from over-matching nested directories such as
  `tie-offs/rig/`, which stay versioned (Knot 0.41.1+; older binaries'
  bare `rig/` line is migrated in place on startup).
- **Pre-existing projects:** if `rig/` files were already tracked by the
  project git before the 0.31.0 layout migration, the `.gitignore` entry
  alone is not enough — the user must run the one-time
  `git rm -r --cached rig/` untrack step (step 4c). Knot detects this
  and logs a warning instead of doing it.
- **Runtime data lives in the project tree** at `tie-offs/<rig>/`
  (tie-offs, the appended service log, event queue, state) and is committed
  with the project's git history by Knot's per-knot-run commits.

---

## Cross-Reference

**Before using other Knot skills:** Read the `knot-abstractions` skill
for the layered architecture overview. Understanding the rig/profile/
skill/application boundary is essential for effective work.

After initialisation, the workflow continues with:

- **knot-abstractions skill** — foundational architecture overview
- **knot-start skill** — start, stop, and restart the service; capture
  the service log
- **knot-create skill** — create looms, knots, and profiles (file-first)
- **knot-dispatch skill** — trigger knots into action
- **knot-inspect skill** — inspect rig, loom, and knot state
- **knot-manage skill** — review completed rig work

This skill prepares the rig and installs Knot skills globally. The
other skills manage the content.

## Global Skill Installation

Knot skills are developed and tested at the project level
(`.agents/skills/`) before being deployed to the production
locations in the personal skills repository (`~/.agents/`):

- Sub-skills → `~/.agents/skills-library/<skill>/` — **not**
  auto-discovered by pi; read on demand via the master's routing
  table or `pi --skill <path>`.
- Master router → `~/.agents/skills/knot/` — the only Knot skill
  that enters the system prompt in other projects.

Step 4a above handles this automatically during initialisation.

To install skills manually (e.g. after updating a skill at project
level):

```bash
# Sub-skills -> production library
for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  mkdir -p ~/.agents/skills-library/$skill
  cp -r .agents/skills/$skill/. ~/.agents/skills-library/$skill/
done
# Copy any extra files (e.g. glossary)
if [ -f .agents/skills/knot-init/knot-glossary.md ]; then
  cp .agents/skills/knot-init/knot-glossary.md \
     ~/.agents/skills-library/knot-init/knot-glossary.md
fi
# Master router -> auto-discovered location
mkdir -p ~/.agents/skills/knot
cp -r .agents/skills/knot/. ~/.agents/skills/knot/
```

Always verify after copying — `cp` can silently fail:

```bash
for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  diff .agents/skills/$skill/SKILL.md \
       ~/.agents/skills-library/$skill/SKILL.md > /dev/null 2>&1 && \
    echo "$skill: OK" || echo "$skill: FAILED"
done
diff .agents/skills/knot/SKILL.md \
     ~/.agents/skills/knot/SKILL.md > /dev/null 2>&1 && \
  echo "knot: OK" || echo "knot: FAILED"
```
