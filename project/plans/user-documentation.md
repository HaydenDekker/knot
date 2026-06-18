# Plan: User Documentation and Documentation Skill

## Problem

Knot is at version 0.12.0 with 36+ completed plans, 7 ADRs, 3 PRDs, and 4 agent skills — but no user-facing documentation. The skills themselves contain the richest material (config reference, quick reference, error handling, workflows) but they're formatted for agent consumption, not human readers. The README is 40 lines. The domain glossary is thorough but not linked to any narrative. Someone new to Knot has nothing to start from.

## Target

A `docs/` directory containing human-facing user documentation derived systematically from existing project artifacts (skills, glossary, PRDs, completed plans), plus a reusable `project-documentation` skill that encodes the extraction methodology for future releases and other projects.

## Implementation Status: ⬜ Draft

## Phases

### Phase 0: Audit Source Material

> **✅ Complete** — 2026-06-18. Audit documented in [user-documentation-audit.md](user-documentation-audit.md).

Map every existing artifact to its target user doc. Identify gaps where user docs need original writing (tutorials, troubleshooting) vs. extraction (reference material from skills).

Source map:

| Source Artifact | Target Doc | Extraction or Original? |
|-----------------|-----------|------------------------|
| `knot-init` skill | `docs/getting-started.md` | Extract + reformat |
| `domain-glossary.md` | `docs/concepts.md` | Extract + narrative |
| `knot-create` skill | `docs/configuration/profiles.md`, `docs/configuration/knots.md`, `docs/configuration/rig-structure.md` | Extract + split |
| `knot-inspect` skill | `docs/troubleshooting.md` | Extract + extend |
| `knot-design` skill | `docs/design-guide.md` | Extract + reformat |
| `knot-init` + `knot-create` quick reference | `docs/getting-started.md` | Extract |
| PRD user stories (completed) | `docs/workflows/review-workflow.md` | Original (from PRD scenarios) |
| PRD user stories (completed) | `docs/workflows/file-generation-workflow.md` | Original (from PRD scenarios) |
| Skill error handling tables | `docs/troubleshooting.md` | Extract |
| `router.rs` (API routes) | `docs/api-reference.md` | Extract |
| Completed plans (master-plan.md) | `docs/release-notes.md` | Extract + group |
| `README.md` | `docs/concepts.md` (philosophy section) | Extract |

### Phase 1: Write Core User Docs ✅ Complete

Create the foundational docs directory and write the human-facing versions:

- [x] Create `docs/` directory structure:
  ```
  docs/
    getting-started.md
    concepts.md
    configuration/
      profiles.md
      knots.md
      rig-structure.md
    workflows/
      review-workflow.md
      file-generation-workflow.md
    api-reference.md
    troubleshooting.md
    design-guide.md
    release-notes.md
  ```

- [x] **`docs/getting-started.md`** — extract from `knot-init` skill: install steps, first rig init, verification, quick reference. Reformat from agent-instruction style to human tutorial style.

- [x] **`docs/concepts.md`** — extract from `domain-glossary.md` and `README.md`: the mental model (rig → loom → knot → strand → tie-off), the philosophy (file-first, version-controllable), the relationship diagram. Add narrative flow rather than term-by-term definitions.

- [x] **`docs/configuration/profiles.md`** — extract from `knot-create` skill: profile file format, frontmatter fields, how profiles are used at processing time, example profiles. Reformat tables and examples for human readability.

- [x] **`docs/configuration/knots.md`** — extract from `knot-create` skill: knot file format, frontmatter fields, directory resolution, example knots, how to add/modify/delete.

- [x] **`docs/configuration/rig-structure.md`** — extract from `knot-create` skill: rig directory tree, how looms are named and discovered, tie-off paths, log locations.

- [x] **`docs/api-reference.md`** — extract from `router.rs` and skill API sections: list all GET endpoints with purpose and example curl commands. Link to Swagger UI.

- [x] **`docs/design-guide.md`** — extract from `knot-design` skill: naming conventions, idempotency, responsibility boundaries, loop design. Reformat for human readers.

### Phase 2: Write Workflow Tutorials ✅ Complete

Create end-to-end tutorials from PRD scenarios — original writing guided by the domain model:

- [x] **`docs/workflows/review-workflow.md`** — "Review all PRDs in your project": from rig init through creating a profile, creating a loom with a review knot, dropping strands, checking tie-offs. Use the PRD scenarios from `prd-knot-skills.md` as the narrative backbone.

- [x] **`docs/workflows/file-generation-workflow.md`** — "Transform source files into structured output": from rig init through creating a generation knot, watching a source directory, collecting tie-offs. Use the PRD scenarios from `prd-ai-driven-file-generation.md`.

### Phase 3: Write Troubleshooting and Release Notes ✅ Complete

- [x] **`docs/troubleshooting.md`** — extract error handling tables from all 4 skills, add common issues (Knot not running, profile not found, strand dir missing, loom not discovered). Organise by symptom → cause → fix.

- [x] **`docs/release-notes.md`** — extract from `master-plan.md` completed plans, grouped by feature area (not chronological). Feature areas: Configuration (file-first, shared profiles, rig switching), Processing (tie-off append, context management, dedup), Observability (rig-log, loom-log, HTTP endpoints, Swagger), Integration (Pi agent, skills, git versioning). Include version number (0.12.0).

- [x] **Update `README.md`** — add a "Documentation" section linking to `docs/`, keep the high-level philosophy and concepts.

### Phase 4: Create `project-documentation` Skill

Package the methodology into a reusable skill at `.agents/skills/project-documentation/SKILL.md`:

- [ ] Write skill frontmatter with metadata (name, description, license, version, USE FOR / DO NOT USE FOR)
- [ ] Document the **source extraction methodology**:
  - Skills → config reference, quick reference, troubleshooting
  - Domain glossary → concepts doc
  - PRD user stories → workflow tutorials
  - Completed plans → release notes / changelog
  - Router/API code → API reference
- [ ] Document the **target doc structure** (the directory layout from Phase 0)
- [ ] Document the **audience split** (end-user docs vs. contributor docs)
- [ ] Document the **extraction workflow** (read source → extract sections → reformat for human audience → write target)
- [ ] Include the **doc quality checklist** (links work, examples are runnable, no internal-only terminology without glossary reference, each doc has a clear audience and goal)

### Phase 5: Publish Skill Globally

- [ ] Copy skill to global skills: `cp -r .agents/skills/project-documentation ~/.agents/skills/project-documentation`
- [ ] Verify skill file is valid (frontmatter parses, no broken relative links)

## Notes

- This is documentation-only work — no code changes, no tests. The plan can work directly on `main` without a feature branch.
- The skill produced in Phase 4 is the lasting deliverable. The docs themselves are the immediate output, but the skill is what makes the process repeatable for future releases and other projects.
- Domain glossary terms used in user docs should link back to `project/domain-glossary.md` for the authoritative definition.
