# Phase 0 Audit — User Documentation Source Material

> **Date:** 2026-06-18
> **Plan:** [user-documentation.md](user-documentation.md) — Phase 0

This audit maps every existing project artifact to its target user doc,
assesses extraction feasibility, and identifies gaps requiring original
writing.

---

## Artifact Inventory

### 1. Agent Skills (`.agents/skills/`)

Four skills, all MIT-licensed, versioned, and targeting Knot 0.3.0+.
Written in agent-instruction format (imperative workflows, API call
sequences, error tables). Rich in reference material; poor for human
narrative.

| Skill | File | Size (approx.) | Key Content |
|-------|------|---------------|-------------|
| `knot-init` | `SKILL.md` | ~40 lines + sections | Rig init workflow, health checks, profile creation from `models.json`, quick reference curl commands, error handling table, API endpoint table |
| `knot-create` | `SKILL.md` | ~40 lines + sections | Full CRUD for looms, knots, profiles. File formats (frontmatter tables), directory resolution, verification endpoints, quick reference shell snippets, error handling table |
| `knot-inspect` | `SKILL.md` | ~40 lines + sections | Read-only inspection workflows, response schemas for all GET endpoints, processing status values, loom event types, error handling table, quick reference |
| `knot-design` | `SKILL.md` | ~40 lines + sections | Idempotency patterns, naming conventions, responsibility boundaries, loop design and breaking patterns, anti-patterns, step-by-step knot design guide |

### 2. Domain Glossary (`project/domain-glossary.md`)

Comprehensive glossary with term definitions and a relationship diagram.
Last updated 2026-06-14. Covers all core concepts: rig, agent profile,
knot, loom, strand directory, prompt template, strand, tie-off,
tie-off directory, loom-log, knot-state, rig-log.

### 3. PRDs (`project/prds/`)

Three PRDs, all marked ✅ Complete.

| PRD | File | Completed | User Stories | Key Scenarios |
|-----|------|-----------|-------------|---------------|
| AI-Driven File Generation | `prd-ai-driven-file-generation.md` | 2026-06-04 | 9 stories | Create knot, watch loom/generate tie-offs, multiple looms, agent runtime config, observe status, processing history, parent directory fan-out, shared profiles, rig switching/sharing |
| Knot Skills | `prd-knot-skills.md` | 2026-06-04 | 4 stories | Init rig, configure looms/knots, inspect rig status, browse Swagger API |
| System Reliability | `prd-system-reliability.md` | exists | unknown | Not read for this audit — not directly referenced in Phase 0 source map |

### 4. Completed Plans (`project/plans/master-plan.md`)

37 plans total: 33 ✅ Complete, 3 ⬜ Planned, 1 ⬜ Planned (superseded).
Plans have detailed "Result" sections describing implementation outcomes.
Plans span from 2026-06-03 to 2026-06-18.

### 5. ADRs (`project/adrs/`)

7 ADRs covering: integration test server pattern, server child tasks,
channel cascade shutdown, shared agent profiles, skill integration
testing, file-first configuration, stdin-only agent invocation.

### 6. DPRs (`project/dprs/`)

1 DPR: `dpr-001-multi-knot-watch-fanout.md`.

### 7. Design Docs (`project/docs/`)

2 design docs: `design-shared-agent-profiles.md`,
`design-tie-off-append-and-event-context.md`.

### 8. README (`README.md`)

40-line file. Covers philosophy (file-first, version-controllable) and
core concepts at a high level. No installation instructions, no
configuration examples, no API reference.

### 9. Router (`src/adapters/inbound/router.rs`)

Defines 11 HTTP endpoints (10 GET, 1 POST), all documented with
utoipa for Swagger UI. Endpoints: health, agents/{dir}, config/rig,
config/reload, looms, looms/{id}, looms/{id}/activity,
looms/{id}/knots, looms/{id}/knots/{name}, profiles, profiles/{name}.

---

## Source-to-Target Mapping

### `docs/getting-started.md` — Extract + Reformat

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-init` skill | Install/build steps, rig init workflow, health check, profile creation from `models.json` | Reformat from agent-instruction ("send GET /health") to human tutorial ("check Knot is running: `curl http://localhost:3000/health`") |
| `knot-init` skill quick reference | curl commands for health, config, profiles, looms | Include as "verify your setup" section |
| `knot-create` skill quick reference | Profile and loom creation shell snippets | Include as "create your first loom" section |
| `README.md` | Philosophy paragraph | Include as "what is Knot" intro |

**Gaps (original writing needed):**
- Installation guide (cargo install, binary distribution) — not in any source
- "First loom" walkthrough that connects init → profile → loom → strand → tie-off as a cohesive narrative
- System requirements (what agent CLI is needed, what LLM provider)

### `docs/concepts.md` — Extract + Narrative

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `domain-glossary.md` | All term definitions, relationship diagram | Restructure from glossary format to narrative: "here is the mental model" with terms introduced in context |
| `README.md` | Philosophy section, core concepts bullets | Integrate as opening narrative |

**Gaps (original writing needed):**
- Narrative flow connecting terms (glossary is term-by-term, needs story)
- Diagram of the processing pipeline (strand event → knot → agent → tie-off)
- "How Knot fits your workflow" section

### `docs/configuration/profiles.md` — Extract + Split

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-create` skill | Agent profile file format, frontmatter table, example profiles, "how profiles are used at processing time" | Straight extraction — content is already structured well |
| `domain-glossary.md` | Agent Profile term definition | Cross-reference |

**Gaps (original writing needed):**
- Minimal — source is comprehensive

### `docs/configuration/knots.md` — Extract + Split

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-create` skill | Knot definition file format, frontmatter table, example knot files, CRUD workflows (create/add/modify/delete), directory resolution | Straight extraction |
| `domain-glossary.md` | Knot, Strand Directory, Prompt Template term definitions | Cross-reference |

**Gaps (original writing needed):**
- Minimal — source is comprehensive

### `docs/configuration/rig-structure.md` — Extract + Split

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-create` skill | Domain model diagram, rig directory tree, loom naming convention, tie-off paths, log locations | Extract from domain model section |
| `domain-glossary.md` | Rig, Loom, Tie-off Directory, Loom-log, Rig-log, Knot-state term definitions | Cross-reference |

**Gaps (original writing needed):**
- Minimal — source is comprehensive

### `docs/troubleshooting.md` — Extract + Extend

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-init` skill error table | 6 error scenarios | Extract, merge with others |
| `knot-create` skill error table | 5 error scenarios | Extract, merge with others |
| `knot-inspect` skill error table | 6 error scenarios | Extract, merge with others |
| `knot-design` skill | KnotParseWarning events, stale-strand checks | Partial — design patterns relevant to troubleshooting loops |

**Gaps (original writing needed):**
- Common issues beyond API errors: Knot not running, profile not found at processing time, strand dir missing, loom not discovered, agent CLI not found, LLM provider auth failures
- "Symptom → Cause → Fix" format is not present in sources — requires original organisation
- Debugging workflow (how to use loom-log and rig-log to diagnose problems)

### `docs/design-guide.md` — Extract + Reformat

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `knot-design` skill | Full content: idempotency, naming, responsibility, loop design, step-by-step guide | Straight extraction — reformat from agent skill to human guide |

**Gaps (original writing needed):**
- Minimal — source is the most comprehensive skill

### `docs/workflows/review-workflow.md` — Original from PRD Scenarios

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `prd-knot-skills.md` Story 1 | Rig init scenario | Provide starting point |
| `prd-knot-skills.md` Story 2 | Configure looms/knots scenarios | Provide knot creation steps |
| `prd-knot-skills.md` Story 3 | Inspect rig status scenarios | Provide verification steps |
| `knot-create` skill | Profile and knot creation examples | Provide concrete file examples |

**Gaps (original writing needed):**
- End-to-end narrative: "Review all PRDs in your project" as a cohesive story
- Connecting the steps: init → profile → loom → strand → check tie-off
- This is primarily original writing guided by PRD scenarios

### `docs/workflows/file-generation-workflow.md` — Original from PRD Scenarios

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `prd-ai-driven-file-generation.md` Story 1 | Create knot and confirm active | Provide starting point |
| `prd-ai-driven-file-generation.md` Story 2 | Watch loom and generate tie-offs | Provide processing steps |
| `prd-ai-driven-file-generation.md` Story 3 | Multiple looms in parallel | Provide scaling example |
| `prd-ai-driven-file-generation.md` Story 5 | Observe status | Provide monitoring steps |
| `knot-create` skill | Profile and knot creation examples | Provide concrete file examples |

**Gaps (original writing needed):**
- End-to-end narrative: "Transform source files into structured output"
- Connecting the steps through a concrete example
- This is primarily original writing guided by PRD scenarios

### `docs/api-reference.md` — Extract from Code + Skills

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `router.rs` | 11 endpoint definitions, method types, path parameters | Extract route table |
| `knot-init` skill API section | Endpoint table, response schemas for health/config/looms/profiles | Extract schemas |
| `knot-create` skill API section | Endpoint table, response schemas for looms/knots/profiles | Extract schemas |
| `knot-inspect` skill API section | Endpoint table, response schemas, processing status values, loom event types | Most comprehensive API section |

**Gaps (original writing needed):**
- Swagger UI reference (`http://localhost:3000/swagger-ui`)
- Organisation: group endpoints by domain (system, config, looms, profiles)
- curl examples for each endpoint (scattered across skills, need consolidation)

### `docs/release-notes.md` — Extract + Group

| Source | Content to Extract | Notes |
|--------|-------------------|-------|
| `master-plan.md` | All 37 plan entries with results | Extract result summaries |
| `master-plan.md` | Version bumps per plan | Extract version progression |

**Gaps (original writing needed):**
- Grouping by feature area (not chronological): Configuration, Processing, Observability, Integration
- Version summary (0.1.0 → 0.12.0)
- This requires original organisation of extracted content

---

## Summary of Gaps

Docs that are **primarily extraction** (minimal original writing):
- `docs/configuration/profiles.md` — comprehensive source in `knot-create` skill
- `docs/configuration/knots.md` — comprehensive source in `knot-create` skill
- `docs/configuration/rig-structure.md` — comprehensive source in `knot-create` + glossary
- `docs/design-guide.md` — comprehensive source in `knot-design` skill

Docs that are **extraction + reformatting** (structural changes needed):
- `docs/getting-started.md` — extract from `knot-init` + `knot-create` quick refs
- `docs/concepts.md` — extract from glossary + README, restructure as narrative
- `docs/api-reference.md` — extract from router.rs + skill API sections, consolidate
- `docs/release-notes.md` — extract from master-plan.md, reorganise by feature

Docs that are **primarily original writing** (guided by sources):
- `docs/workflows/review-workflow.md` — PRD scenarios provide backbone but narrative is original
- `docs/workflows/file-generation-workflow.md` — PRD scenarios provide backbone but narrative is original
- `docs/troubleshooting.md` — error tables are extracted but "symptom → cause → fix" organisation and common issues are original

## Missing Source Material

Topics where no adequate source exists:
- **Installation guide** — README mentions `cargo build` and `cargo run` but no install instructions
- **System requirements** — no source documents prerequisites (Rust toolchain, agent CLI, LLM provider)
- **Troubleshooting common issues** — skills cover API errors but not runtime issues (agent not found, LLM auth, disk space, permissions)
- **Tutorial narrative** — no end-to-end walkthrough exists; PRD scenarios provide steps but not a cohesive story

These gaps will require original writing in Phases 1–3.
