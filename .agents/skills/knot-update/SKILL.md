---
name: knot-update
description: "Record format changes between Knot binary versions. When a project updates its Knot binary, this skill tells the agent what changed in project documents (profiles, knots, looms) and how to migrate them. Contains a versioned changelog with migration instructions for each breaking change. USE FOR: update knot, knot version change, knot migration, knot changelog, migrate knot documents, knot format change, knot upgrade, knot breaking change, profile format change, knot file migration, loom migration. DO NOT USE FOR: creating looms (use knot-create), modifying looms (use knot-create), initialising a rig (use knot-init), inspecting state (use knot-inspect), fixing bugs (use project-bugfix)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.18.0+"
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
