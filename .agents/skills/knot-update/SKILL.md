---
name: knot-update
description: "Record format changes between Knot binary versions. When a project updates its Knot binary, this skill tells the agent what changed in project documents (profiles, knots, looms) and how to migrate them. Contains a versioned changelog with migration instructions for each breaking change. USE FOR: update knot, knot version change, knot migration, knot changelog, migrate knot documents, knot format change, knot upgrade, knot breaking change, profile format change, knot file migration, loom migration. DO NOT USE FOR: creating looms (use knot-create), modifying looms (use knot-create), initialising a rig (use knot-init), inspecting state (use knot-inspect), fixing bugs (use project-bugfix)."
license: MIT
metadata:
  author: Knot Team
  version: "1.4.0"
  compatibility: "Knot 0.23.0+"
---

# Knot Update Skill

Record and communicate format changes between Knot binary versions. When a
project updates its Knot installation, this skill provides the agent with a
versioned changelog of document format changes and step-by-step migration
instructions.

This is a **reference skill** — it does not define a workflow to execute.
Instead, it is read when a project updates Knot so the agent knows what
document changes are required.

---

## Core Philosophy

### Format Changes Are Breaking by Default

Knot reads `.md` files with YAML frontmatter. Any change to the
frontmatter schema or body semantics is a breaking change for existing
project documents. Projects that have been running with Knot will have
documents in the old format that must be migrated.

This skill ensures:

- **Every version is documented** — even small format tweaks
- **Migration is mechanical** — search patterns and replacements are
  explicit, not described in prose alone
- **Projects can self-serve** — the agent reads this skill and applies
  migrations without external guidance

### How This Skill Is Used

1. A project updates its Knot binary (e.g. `cargo install --path .` or
   downloads a new release).
2. The agent reads this skill file to see what changed since the
   project's last Knot version.
3. For each changelog entry newer than the project's current version,
   the agent applies the migration instructions to the project's
   `rig/profiles/*.md` and `rig/*-loom/*.md` files.
4. The agent verifies the migrated files by checking `rig/state.json`
   (Knot must be running).

---

## Changelog

Entries are listed newest first. Each entry specifies the Knot version,
date, and migration instructions for affected document types.

---

### 0.26.0 — Tie-Off Filenames Renamed (2026-07-13)

Tie-off output files renamed from `{knot-name}-tie-off.md` to
`tie-off-{knot-name}.md`.

**Why:** The `tie-off-` prefix groups tie-off files together in
`rig/tie-offs/{loom-id}/`, making them visually distinct from event
subdirectories and other files that may appear in the same directory.

#### Affected Files

| What Changed | Old Filename | New Filename |
|---|---|---|
| Tie-off files | `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md` | `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md` |

#### Migration Steps

Tie-off files are append-mode history. Existing files at the old name
are harmless — Knot will create new files with the renamed path on the
next processing event. To migrate existing history:

1. **Find all tie-off files:**
   ```bash
   find rig/tie-offs/ -type f -name '*-tie-off.md'
   ```

2. **Rename each file** — move the knot name from prefix to suffix:
   ```bash
   for file in rig/tie-offs/*-loom/*-tie-off.md; do
     name="${file##*/}"                          # filename with extension
     name="${name%-tie-off.md}"                   # strip suffix → knot name
     dir="${file%/*}"                             # directory
     mv "$file" "${dir}/tie-off-${name}.md" 2>/dev/null
   done
   ```

   Or use a find+rename one-liner:
   ```bash
   find rig/tie-offs/ -type f -name '*-tie-off.md' | while read file; do
     base="${file##*/}"                          # filename with extension
     base="${base%-tie-off.md}"                   # strip suffix → knot name
     dir="${file%/*}"                             # directory
     mv "$file" "${dir}/tie-off-${base}.md" 2>/dev/null
   done
   ```

3. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that knot `last_tie_off_path` values use the new `tie-off-{knot-name}.md` format.

#### If Not Migrated

- Old-named files remain on disk (harmless orphan files)
- New processing events create tie-off files at the new filename
- No data loss — append-mode means existing content in old files is preserved
- Knot's `state.json` will reference the new filenames going forward

#### Fields Unchanged by This Migration

Only the filesystem filename of tie-off output files is affected.
All profile and knot frontmatter fields, knot definitions, and
loom definitions are unchanged.

---

### 0.24.0 — StrandSource: Unified Input Direction (2026-07-10)

The `listens-for` array is replaced by `strand-source` (expressed as
`strand-dir` with an `event:` URI). Each knot now has exactly **one**
input direction, restoring the "one strand, one direction" principle.
Event consumer knots set `strand-dir` to an `event:` URI instead of
a filesystem path, and an optional `event-description` field provides
the semantic contract injected into the producer's prompt.

#### Affected Documents

| Document Type | Location | Change |
|---------------|----------|--------|
| Knot | `rig/*-loom/*.md` | `listens-for` removed; `strand-dir` now accepts `event:` URIs; new optional `event-description` field |

#### Frontmatter Changes

**Before (dual input — filesystem path + event intents):**

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "project/reviews"
listens-for:
  - target-knot: quality-reviewer
    event-id: ReviewCompleted
    event-description: >
      Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

**After (single input — event URI replaces listens-for):**

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "event:quality-reviewer:ReviewCompleted"
event-description: >
  Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

**Normal (non-event) knots are unchanged:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section.
```

#### Migration Steps

1. **Find knots with `listens-for`:**
   ```bash
   grep -rl "listens-for:" rig/ 2>/dev/null
   ```

2. **For each consumer knot with `listens-for`:**
   a. Read the knot file.
   b. For each intent in `listens-for`, take the `target-knot` value
      (the producer) and the `event-id` value.
   c. Replace `strand-dir` with:
      `"event:<target-knot>:<event-id>"`
   d. Move `event-description` from the intent object into a top-level
      `event-description` frontmatter field.
   e. Remove the entire `listens-for:` block from the frontmatter.

   **If a knot has `listens-for` with multiple intents**, it cannot
   be migrated directly — `StrandSource` supports only one input.
   Create separate knots, one per event subscription.

3. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that consumer knots appear without errors and that `listens-for`
   is no longer present in any knot definition.

#### If Not Migrated

- Knots with `listens-for` will parse with an **unknown-property warning**
  (`listens-for` is no longer a recognised frontmatter key).
- The knot will not have event subscriptions (the `listens-for` entries
  are silently ignored).
- The knot's `strand-dir` filesystem path still works normally.
- No data loss — existing tie-off files and event directories are
  preserved.

#### Fields Unchanged by This Migration

All other frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes (now also accepts `event:` URIs) |
| Knot | `git-versioned` | Yes |

---

### 0.23.0 — Intent-Based Event Routing (2026-07-09)

Intent-based event routing adds first-class agent-to-agent events.
Consumer knots declare `listens-for` intents in their frontmatter;
Knot injects event instructions into producer prompts and dispatches
matching events to consumers at runtime.

#### Affected Documents

| Document Type | Location | Change |
|---------------|----------|--------|
| Knot | `rig/*-loom/*.md` | New optional field: `listens-for` |

#### New Frontmatter Field: `listens-for`

Knots can now declare event intents using the `listens-for` YAML list:

```markdown
---
name: refactor-planner
agent-profile-ref: coder
strand-dir: "../../tie-offs/review-loom/ReviewCompleted/"
listens-for:
  - target-knot: quality-reviewer
    event-id: ReviewCompleted
    event-description: >
      Emitted when a quality review is complete.
---

Create a refactor plan when a quality review is complete.
```

Each intent entry has three fields:

| Field | Required | Description |
|-------|----------|-------------|
| `target-knot` | Yes | Which knot may emit this event (knot name) |
| `event-id` | Yes | Unique event identifier (e.g. `ReviewCompleted`, `PlanCreated`) |
| `event-description` | Yes | When the event fires and what data it should contain |

**Consumer `strand-dir`:** When using intent-based routing, the
consumer's `strand-dir` should point to the event subdirectory:
`../../tie-offs/{loom-id}/{event-id}/`. Knot creates event files
at this location when matching events are dispatched.

**Producer side:** Producers have no frontmatter changes. Knot
automatically injects event instructions into the producer's prompt
by scanning all consumers' `listens-for` declarations.

#### Migration Steps

No migration required for existing rigs. This is a new optional feature:

1. Existing knots without `listens-for` behave exactly as before.
2. To adopt intent-based routing, add `listens-for` to consumer knots.
3. Update consumer `strand-dir` to the intent event subdirectory.
4. Producers automatically receive event instructions (no config change).

#### Fields Unchanged by This Migration

All existing frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes |
| Knot | `git-versioned` | Yes |

---

### 0.22.0 — Tie-Off Paths Flattened (2026-07-01)

Tie-off files moved from nested knot subdirectories to flat files directly
under the loom's tie-off directory.

**Why:** The intermediate `{knot-name}` subdirectory added no value — the
tie-off filename already identifies the knot. Flattening frees the loom-level
directory for event capture subdirectories.

#### Affected Files

| What Changed | Old Path | New Path |
|---|---|---|
| Tie-off files | `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` | `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md` |
| Loom-log | `rig/tie-offs/{loom-id}/.loom-log` | `rig/tie-offs/{loom-id}/.loom-log` (unchanged) |

#### Migration Steps

Tie-off files are append-mode history. Existing files at the old nested
location are harmless orphans — Knot will create new flat files on the next
processing event. To consolidate:

1. **Find old nested tie-off directories:**
   ```bash
   find rig/tie-offs/ -mindepth 2 -type d -name '*-tie-off.md' -prune -o -mindepth 2 -type d -print
   ```
   Or more simply, list knot-name subdirectories:
   ```bash
   find rig/tie-offs/ -mindepth 2 -maxdepth 2 -type d
   ```

2. **Move each tie-off file flat:**
   ```bash
   for knot_dir in rig/tie-offs/*-loom/*/; do
     knot_name=$(basename "$knot_dir")
     mv "$knot_dir/${knot_name}-tie-off.md" "${knot_dir%/*}/${knot_name}-tie-off.md" 2>/dev/null
     rmdir "$knot_dir" 2>/dev/null
   done
   ```

3. **Or bulk-move all at once:**
   ```bash
   find rig/tie-offs/ -mindepth 2 -maxdepth 2 -type d | while read dir; do
     name=$(basename "$dir")
     target="$dir/../${name}-tie-off.md"
     if [ -f "${dir}/${name}-tie-off.md" ]; then
       mv "${dir}/${name}-tie-off.md" "$target"
       rmdir "$dir"
     fi
   done
   ```

4. **Verify** — Knot must be running:
   ```bash
   cat rig/state.json | python3 -m json.tool
   ```
   Check that knot `last_tie_off_path` values use the flat structure.

#### If Not Migrated

- Old nested files remain on disk (harmless orphan files)
- New processing events create tie-off files at the new flat location
- No data loss — append-mode means existing content in old files is preserved
- Knot's `state.json` will reference the new flat paths

#### Fields Unchanged by This Migration

All profile and knot frontmatter fields are unchanged. Only the
filesystem location of tie-off output files is affected.

---

### 0.18.0 — Prompt text moved to markdown body (2026-06-24)

Prompt content moved from YAML frontmatter block scalars to the markdown
body (text after the closing `---`). Frontmatter now holds only structural
metadata.

**Why:** Prompt text in YAML frontmatter is indentation-sensitive, produces
noisy diffs, and inverts the normal markdown convention where the body
holds the primary content.

#### Affected Documents

| Document Type | Location | Old Field | New Location |
|---------------|----------|-----------|--------------|
| Agent Profile | `rig/profiles/*.md` | `profile-prompt: \|` in frontmatter | Markdown body after closing `---` |
| Knot | `rig/*-loom/*.md` | `prompt-template:\n  instructions: \|` in frontmatter | Markdown body after closing `---` |

#### Profile Migration

**Before:**

```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - read
  - bash
profile-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

A fast reviewer profile.
```

**After:**

```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - read
  - bash
---

You are a fast reviewer. Keep responses concise and direct.
```

**Migration steps:**

1. Read the profile file at `rig/profiles/{name}.md`.
2. Extract the text value of `profile-prompt` (the full block scalar,
   unindented).
3. Remove the `profile-prompt` line and its block content from the
   frontmatter.
4. Replace the markdown body (everything after closing `---`) with the
   extracted prompt text. If there was a heading or summary in the
   old body, discard it — it was documentation that duplicated the
   prompt.
5. If the prompt text is long, it becomes the entire body. No heading
   wrapper needed — the body *is* the prompt.

**Search pattern:** Look for `profile-prompt: |` in any `.md` file
under `rig/profiles/`.

#### Knot Migration

**Before:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
prompt-template:
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
---

# Goals Review

Review the goals section of this PRD.
```

**After:**

```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section of this PRD. Check that:
- Each goal is specific and measurable
- Goals align with the problem statement
```

**Migration steps:**

1. Read the knot file at `rig/{loom-id}/{knot-name}.md`.
2. Extract the text value of `prompt-template.instructions` (the full
   block scalar, unindented).
3. Remove the entire `prompt-template:` block (both `prompt-template:`
   and `  instructions: |` lines) from the frontmatter.
4. Replace the markdown body with the extracted instruction text.
   Discard any old body heading or summary — it was duplicate
   documentation.
5. If the instructions contain multiple paragraphs or lists, they
   become the body as-is (no wrapping heading).

**Search pattern:** Look for `prompt-template:` followed by
`  instructions: |` in any `.md` file under `rig/`.

#### Fields Unchanged by This Migration

These frontmatter fields keep the same meaning and location:

| Document | Field | Unchanged |
|----------|-------|-----------|
| Profile | `name` | Yes |
| Profile | `provider` | Yes |
| Profile | `model` | Yes |
| Profile | `tools` | Yes |
| Profile | `timeout` | Yes |
| Knot | `name` | Yes |
| Knot | `agent-profile-ref` | Yes |
| Knot | `strand-dir` | Yes |
| Knot | `git-versioned` | Yes |

---

## Agent Workflow

When a project updates Knot:

1. **Read this skill file** to see the full changelog.
2. **Determine the project's current Knot version** — check any
   `Cargo.lock`, `Cargo.toml`, or project notes for the previous
   version.
3. **For each changelog entry newer than the current version:**
   a. Read the migration instructions for that entry.
   b. Find affected files using the search patterns documented in the
      entry.
   c. Apply the transformations (edit frontmatter, move content to body).
   d. Verify the files parse correctly by restarting Knot and checking
      `rig/state.json` for errors.
4. **Report migration results** — list each migrated file and confirm
   Knot is reading them without errors.

---

## Adding New Changelog Entries

When Knot introduces a new format change:

1. Add a new versioned entry at the **top** of the Changelog section
   (newest first).
2. Include:
   - Version number and short description as an `###` heading
   - "Why" rationale in one paragraph
   - "Affected Documents" table mapping old fields to new locations
   - Migration steps for each affected document type (before/after
     examples + numbered steps + search patterns)
   - "Fields Unchanged" table to confirm what stays the same
3. Bump the skill `version` in the frontmatter metadata.
4. Publish the updated skill globally:
   ```bash
   cp -r .agents/skills/knot-update ~/.agents/skills/knot-update
   ```

---

## Quick Reference

```bash
# Find profiles using old format (profile-prompt in frontmatter)
grep -rl "profile-prompt:" rig/profiles/ 2>/dev/null

# Find knots using old format (prompt-template in frontmatter)
grep -rl "prompt-template:" rig/ 2>/dev/null

# Publish updated skill globally
cp -r .agents/skills/knot-update ~/.agents/skills/knot-update

# Verify Knot is reading migrated files
cat rig/state.json | python3 -m json.tool
```

---

## Cross-Reference

Related skills:

1. **knot-create skill** — create and modify looms, knots, and profiles
2. **knot-inspect skill** — inspect rig state after migration
3. **knot-init skill** — initialise a new rig (no migration needed)

This skill records **what changed** between Knot versions. The other
skills define **how to work with** the current Knot format.
