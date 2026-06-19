# Workflows: File Generation Workflow

A file generation workflow uses Knot to automatically create or update
project files based on source documents. This pattern drives
documentation generation, plan creation, and other automated outputs.

## Example: PRD-Driven Plan Generation

In this example, a knot watches a PRD directory and generates
implementation plans from PRD documents.

### 1. Create a Profile

File generation can be a long-running task. Set a higher timeout:

`rig/profiles/planner.md`:

```yaml
---
name: planner
provider: openai
model: gpt-4o
tools:
  - fs
timeout: 600
system-prompt: |
  You are a project planning agent. Create detailed, actionable
  implementation plans from product requirements.
---

# Planner Profile

Profile for plan generation with extended timeout and filesystem access.
```

### 2. Create the Loom

```bash
mkdir -p rig/planning-loom
```

### 3. Create the Generation Knot

`rig/planning-loom/prd-planner.md`:

```yaml
---
name: prd-planner
agent-profile-ref: planner
strand-dir: "project/prds"
prompt-template:
  instructions: |
    Create an implementation plan from this PRD.

    1. Read the PRD (provided as input).
    2. Inspect project/plans/ for existing plans related to this PRD.
    3. If a plan already exists, update it in place to align with
       the current PRD. If not, create a new plan file.
    4. The plan should have phases, each with clear deliverables.
    5. Write the plan to project/plans/<slug>.md where <slug>
       is derived from the PRD title.

    ## Constraints
    - Never delete existing plan content without a clear reason.
    - If the PRD and plan are already aligned, make no changes.
    - Re-running this on the same PRD must produce no additional changes.
---

# PRD Planner Knot

Generates implementation plans from PRD documents.
```

### 4. Trigger Plan Generation

When a PRD is placed or updated in `project/prds/`, the knot triggers
and the agent creates or updates the corresponding plan in
`project/plans/`.

```bash
cat > project/prds/auth-redesign.md << 'EOF'
# Auth Redesign

## Problem
Current auth flow is slow and confusing.

## Goals
- Reduce login steps from 5 to 2
- Add SSO support
EOF
```

The knot reads this PRD and generates `project/plans/auth-redesign.md`.

### 5. Verify the Output

Check the tie-off to see what the agent did:

```bash
cat rig/tie-offs/planning-loom/prd-planner/prd-planner-tie-off.md
```

## Designing Idempotent Generation

File generation knots must be **idempotent** — running them twice on
the same input should produce the same result. Design instructions that
are **goal-focused, not step-focused**:

### Goal-Focused (Idempotent)

```
Ensure the plan file exists at project/plans/<slug>.md and contains
phases that deliver the PRD's goals. Inspect the file first. If it
already contains aligned phases, make no changes.
```

### Step-Focused (Not Idempotent)

```
1. Create the plan file
2. Add phase 0
3. Add phase 1
```

If the strand re-triggers, the step-focused version appends duplicate
phases. The goal-focused version checks current state first.

## Bidirectional Workflows (Feedback Loops)

When you have knots flowing in opposite directions, you create a
**feedback loop**:

```
PRD change → prd-planner updates plan
Plan change → plan-reviewer updates PRD references
PRD change → prd-planner updates plan
...
```

This is correct and expected. The loop converges when both sides agree
(no more changes needed). See the [Design Guide](../design-guide.md)
for loop design patterns.

## See Also

- [Configuration: Profiles](../configuration/profiles.md) — Timeout and tools
- [Configuration: Knots](../configuration/knots.md) — Knot file format
- [Design Guide](../design-guide.md) — Idempotency and loop design
