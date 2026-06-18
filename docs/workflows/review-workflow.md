# Workflows: Review Workflow

A review workflow uses Knot to automatically review documents as they
change. This is one of the most common patterns — a knot watches a
directory and critiques files using an AI agent.

## Example: PRD Section Reviews

In this example, we set up a loom that reviews different sections of
Product Requirement Documents (PRDs).

### 1. Create a Profile

`rig/profiles/reviewer.md`:

```yaml
---
name: reviewer
provider: openai
model: gpt-4o
system-prompt: |
  You are a thorough technical reviewer. Analyse documents
  carefully and provide specific, actionable feedback.
---

# Reviewer Profile

Profile for detailed document reviews.
```

### 2. Create the Loom

```bash
mkdir -p rig/prd-review-loom
```

### 3. Create Review Knots

Each knot reviews a different section. They share the same strand
directory and profile.

**Goals review** — `rig/prd-review-loom/goals-review.md`:

```yaml
---
name: goals-review
agent-profile-ref: reviewer
strand-dir: "project/prds"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
    - Success criteria are defined
---

# Goals Review Knot
```

**Non-goals review** — `rig/prd-review-loom/non-goals-review.md`:

```yaml
---
name: non-goals-review
agent-profile-ref: reviewer
strand-dir: "project/prds"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the non-goals section. Check that:
    - Scope boundaries are clearly defined
    - Exclusions are justified
---

# Non-Goals Review Knot
```

### 4. Trigger Reviews

Place a PRD in `project/prds/`:

```bash
cat > project/prds/my-feature.md << 'EOF'
# My Feature PRD

## Goals
- Improve load time by 50%
- Reduce server costs

## Non-Goals
- Mobile app support
- Internationalisation
EOF
```

Both knots trigger on this file change. Each reviews its assigned
section and appends feedback to its tie-off file.

### 5. Review the Results

Read the tie-off files:

```bash
cat rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md
cat rig/tie-offs/prd-review-loom/non-goals-review/non-goals-review-tie-off.md
```

Or check via the API:

```bash
curl http://localhost:3000/looms/prd-review-loom/knots/goals-review
```

## Idempotent Reviews

Reviews should be idempotent. If the same file is reviewed twice (e.g.
after a minor edit), the knot should produce a fresh review — not
duplicate previous feedback. Since tie-off files are append-only, each
run adds a new section. The knot's instructions should focus on
reviewing the **current state** of the document, not accumulating
feedback.

Good instructions:

```yaml
instructions: |
  Review the goals section of this document.
  Provide feedback on the current content only.
```

Avoid instructions that say "add to previous feedback" or "continue
where you left off" — these break idempotency.

## See Also

- [Configuration: Knots](../configuration/knots.md) — Knot file format
- [Design Guide](../design-guide.md) — Idempotency best practices
