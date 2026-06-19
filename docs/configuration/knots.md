# Configuration: Knots

Knots are the core processing units of Knot. Each knot defines a single
processing task: which agent runs, what input to watch, and how to
process files.

Knots are `.md` files with YAML frontmatter inside a loom directory
(e.g. `rig/prd-review-loom/goals-review.md`).

## File Format

```yaml
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

# Goals Review Knot

Reviews the goals section of PRD documents.
```

## Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Unique knot identifier within its loom. Becomes the `KnotId`. |
| `agent-profile-ref` | Yes | Name of the agent profile to use. Must match a profile in `rig/profiles/{name}.md`. |
| `strand-dir` | Yes | Directory to watch for strand files. Resolved relative to the project root. |
| `prompt-template.instructions` | Yes | Task-specific instructions. Appended to the profile's system prompt at processing time. |

## Directory Resolution

- `strand-dir` is **relative to the project root** — the directory
  containing `rig/`.
- Absolute paths are also accepted and used as-is.
- Tie-off paths are statically derived:
  `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md`

### Example Layout

```
project_root/
├── project/prds/                    ← strand-dir: "project/prds"
└── rig/
    ├── profiles/
    │   └── fast.md
    ├── tie-offs/
    │   └── prd-review-loom/
    │       ├── .loom-log
    │       └── goals-review/
    │           └── goals-review-tie-off.md
    └── prd-review-loom/             ← loom directory
        └── goals-review.md          ← knot definition
```

## Managing Knots

### List Knots in a Loom

```bash
curl http://localhost:3000/looms/prd-review-loom/knots
```

Returns: `["goals-review", "non-goals-review"]`

### Get Loom Details (Includes All Knots)

```bash
curl http://localhost:3000/looms/prd-review-loom
```

Returns full loom configuration including all knot definitions.

### Check a Knot's Processing Status

```bash
curl http://localhost:3000/looms/prd-review-loom/knots/goals-review
```

Returns the knot's current status (`idle`, `processing`, `completed`,
or `failed`), last processed strand, and tie-off path.

### Create a New Knot

Write a `.md` file inside an existing loom directory:

```bash
cat > rig/prd-review-loom/non-goals-review.md << 'EOF'
---
name: non-goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
prompt-template:
  instructions: |
    Review the non-goals section for clarity.
---

# Non-Goals Review Knot
EOF
```

Knot discovers the new file automatically.

### Modify a Knot

Edit the `.md` file directly. Changes are picked up on the next
processing cycle.

### Delete a Knot

Remove its `.md` file:

```bash
rm rig/prd-review-loom/non-goals-review.md
```

Knot discovers the removal automatically.

## Multiple Knots Per Loom

A single loom can contain multiple knot files, each watching the same or
different strand directories. This is useful when different aspects of
the same input need different treatment:

```
rig/planning-loom/
├── prd-planner.md        ← creates plans from PRDs
├── adr-planner.md        ← aligns plans with ADRs
└── plan-reviewer.md      ← reviews plan quality
```

All three knots live in the same loom (`planning-loom`) because they
all produce or maintain plans.

## Error Handling

| Scenario | Symptom | Fix |
|----------|---------|-----|
| Profile not found | Knot fails with `ProfileNotFound` | Create the profile at `rig/profiles/{name}.md` |
| Strand dir missing | Knot registers but finds no files | Create the directory or fix the path |
| Invalid frontmatter | Knot is skipped; `KnotParseWarning` in activity log | Check YAML syntax in the `.md` file |
| Duplicate knot name | Second knot overwrites first in the loom | Use unique names within each loom |

Check the loom activity log to diagnose issues:

```bash
curl http://localhost:3000/looms/prd-review-loom/activity
```
