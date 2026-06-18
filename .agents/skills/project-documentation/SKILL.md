---
name: project-documentation
description: "Generate user-facing documentation from project artifacts (skills, glossary, PRDs, plans, API code). Extracts configuration reference, concept guides, workflow tutorials, API reference, and release notes from internal project documents and reformats them for human readers. USE FOR: generate docs, write documentation, user docs, doc generation, extract documentation, documentation from skills, docs from PRD, docs from plans, API reference docs, release notes, changelog generation, documentation methodology, doc extraction, write user guide, create documentation, documentation skill. DO NOT USE FOR: writing ADRs (use architecture-decision-record), writing DPRs (use design-pattern-record), creating plans (use project-planner), implementing features, writing code, running tests."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Any project with structured artifacts (skills, PRDs, plans, glossary)"
---

# Project Documentation Skill

Generate user-facing documentation by systematically extracting and
reformatting content from existing project artifacts. This skill encodes
the methodology used to produce the Knot user documentation suite and is
reusable for any project that maintains structured internal documents.

The core insight: **the best source material for user docs already exists**
in the project — it just needs translation from agent-consumption format
to human-reader format.

---

## Core Philosophy

### Derive, Don't Invent

User documentation should be extracted from existing artifacts, not
written from scratch. The project already contains the truth in the form
of:

- **Agent skills** — contain config formats, workflows, error handling,
  and quick-reference commands
- **Domain glossaries** — contain the vocabulary and mental model
- **PRDs** — contain user stories that become tutorial workflows
- **Completed plans** — contain the history of what was built
- **API/router code** — contains the definitive list of endpoints

Writing docs from scratch duplicates effort and drifts from reality.
Extracting from sources keeps docs accurate and maintainable.

### Audience Matters

Internal artifacts (skills, ADRs, plans) are written for agents or
contributors. User docs are written for **end users** who need to
accomplish tasks. The extraction process must translate:

- Agent instructions ("when asked to X, send GET /Y") into user guides
  ("to check status, visit http://localhost:3000/looms")
- Technical implementation details into conceptual explanations
- Error handling tables into troubleshooting guides with symptoms and fixes

### Every Doc Has a Source

Each target document should trace back to one or more source artifacts.
If a doc topic has no source artifact, it needs **original writing**
guided by the domain model — not guesswork. The plan should flag which
docs are extraction vs. original.

---

## Source Extraction Methodology

Map each source artifact type to its target documentation category.
This mapping determines what to extract and where it goes.

### Skills → Configuration Reference, Quick Reference, Troubleshooting

Agent skills are the richest source of extractable documentation. Each
skill typically contains:

| Skill Section | Target Doc | Transformation |
|---------------|-----------|----------------|
| File format examples (frontmatter fields, YAML schemas) | Configuration reference | Reformat tables and examples for human readability. Remove agent-specific instruction text. Keep file paths, field descriptions, and examples. |
| Quick reference (curl commands, shell snippets) | Quick reference section in getting-started | Extract verbatim. These are already user-facing. |
| Error handling tables | Troubleshooting guide | Transform from "scenario → action" to "symptom → cause → fix". Add common issues not covered by the skill (e.g. service not running, permission errors). |
| API endpoint tables | API reference | Extract endpoint descriptions. Cross-reference with actual router code for completeness. |
| Domain model diagrams | Concepts guide | Extract the mental model (hierarchy, relationships). Add narrative flow. |
| Agent workflow steps | Workflow tutorials (partial) | Extract the sequence of operations. Rewrite as user narrative ("you create a profile, then a loom, then trigger processing"). |

**Extraction rule:** Remove all agent-directed language ("when asked to
X, do Y"). Replace with user-directed language ("to do X, follow these
steps").

### Domain Glossary → Concepts Document

The domain glossary provides the vocabulary and mental model. Transform
it from a term-by-term definition list into a **narrative concept guide**:

1. Start with the **top-level mental model** (what is this system, at a
   high level)
2. Introduce the **hierarchy** (rig → loom → knot → strand → tie-off)
   as a story, not a list
3. Weave individual term definitions into the narrative where they
   naturally fit
4. Add a **philosophy section** from the project README (why this design,
   what problems it solves)
5. Link back to the authoritative glossary for the full definitions

**Extraction rule:** The concepts doc is a guided tour, not a dictionary.
Every term should appear in context, not in isolation.

### PRD User Stories → Workflow Tutorials

PRDs define user stories with scenarios. These become **end-to-end
workflow tutorials**:

1. Take the user story as the tutorial title and goal
2. Use the PRD scenario steps as the tutorial structure
3. Fill in the technical details from the relevant skills (file formats,
   API calls, verification steps)
4. Write in second person ("you create a profile...")
5. Include verification steps after each major action

**Extraction rule:** The PRD provides the skeleton; the skills provide
the muscle. Cross-reference both.

### Completed Plans → Release Notes / Changelog

Completed plans in `master-plan.md` provide the feature history.
Transform them from chronological plan entries into **feature-grouped
release notes**:

1. Read `master-plan.md` for all completed plans
2. Group plans by **feature area** (not chronological):
   - Configuration (file-first, shared profiles, rig switching)
   - Processing (tie-off append, context management, dedup)
   - Observability (rig-log, loom-log, HTTP endpoints, Swagger)
   - Integration (Pi agent, skills, git versioning)
3. For each group, summarise what was delivered (not how)
4. Include the current version number
5. Link to relevant docs for each feature area

**Extraction rule:** Release notes answer "what can I do now?" not
"what did we build last Tuesday."

### Router/API Code → API Reference

The router source code is the definitive list of available endpoints.
Extract from code, not from skills (which may be incomplete):

1. Read the router file (e.g. `src/router.rs`)
2. Extract each route: method, path, handler name
3. Cross-reference with skill API sections for descriptions and examples
4. Add `curl` examples for each endpoint
5. Link to Swagger/OpenAPI UI if available

**Extraction rule:** The code is authoritative. Skills provide
descriptions and examples. If a route exists in code but not in any
skill, include it with a placeholder description.

---

## Target Doc Structure

Organise user documentation in a `docs/` directory with this layout:

```
docs/
├── getting-started.md          ← installation, first rig, quick reference
├── concepts.md                 ← mental model, terminology, philosophy
├── configuration/
│   ├── profiles.md             ← agent profile format and usage
│   ├── knots.md                ← knot definition format and usage
│   └── rig-structure.md        ← rig directory tree and conventions
├── workflows/
│   ├── review-workflow.md      ← example: review documents workflow
│   └── file-generation-workflow.md ← example: file generation workflow
├── api-reference.md            ← HTTP endpoints with curl examples
├── troubleshooting.md          ← symptom → cause → fix guide
├── design-guide.md             ← naming, idempotency, loop design
└── release-notes.md            ← feature-grouped changelog
```

### Doc Purposes

| Document | Audience | Goal |
|----------|----------|------|
| `getting-started.md` | New users | Get a rig running in under 5 minutes |
| `concepts.md` | New users | Understand the mental model before configuring anything |
| `configuration/profiles.md` | Users creating profiles | Know the file format, fields, and how profiles are used |
| `configuration/knots.md` | Users creating knots | Know the file format, fields, and how knots work |
| `configuration/rig-structure.md` | Users understanding the rig | Know the directory layout and naming conventions |
| `workflows/*.md` | Users building real workflows | Follow an end-to-end example |
| `api-reference.md` | Power users, integrators | Know all available HTTP endpoints |
| `troubleshooting.md` | All users | Diagnose and fix common problems |
| `design-guide.md` | Advanced users, designers | Design correct, idempotent knots |
| `release-notes.md` | All users | Know what features exist and what's new |

---

## Audience Split

User documentation serves two audiences with different needs. Each
document should state its audience and be written accordingly.

### End-User Docs

**Audience:** Developers using the tool to accomplish tasks. They don't
care about implementation details — they want to get things done.

**Characteristics:**
- Task-oriented ("how do I create a profile?")
- Minimal jargon; domain terms linked to concepts doc
- Examples are runnable and tested
- Focus on the happy path; edge cases in troubleshooting
- No internal terminology without glossary reference

**Docs in this category:** getting-started, configuration, workflows,
troubleshooting, release-notes.

### Contributor Docs

**Audience:** Developers who maintain or extend the project. They need
to understand design decisions and architecture.

**Characteristics:**
- Design-oriented ("why is it structured this way?")
- Uses domain terminology freely (with glossary reference)
- Links to ADRs and DPRs for decisions and patterns
- Explains trade-offs and constraints
- May include code references

**Docs in this category:** design-guide, api-reference (also serves
end-users), concepts (bridges both audiences).

### Bridging Document

`concepts.md` serves as the bridge between audiences. It introduces the
mental model for end-users while being thorough enough for contributors.
It links to the glossary for definitions and to the design guide for
deeper architectural understanding.

---

## Extraction Workflow

Follow this sequence for each target document:

### Step 1: Read Source Artifacts

Read all relevant source artifacts in full. Don't skim — the extraction
depends on understanding the complete content. Note which sections map
to which target document.

**Example for `configuration/knots.md`:**
- Read `knot-create` skill (knot file format section)
- Read `knot-design` skill (naming conventions, responsibility)
- Note which content is extraction vs. what needs original writing

### Step 2: Extract Sections

For each source section identified:

1. **Identify the content type:**
   - Reference material (file formats, field tables) → extract directly
   - Procedures (agent workflows) → rewrite as user steps
   - Explanations (design principles) → reformat for human narrative

2. **Extract the factual content:**
   - File paths, field names, field descriptions
   - Example files and their frontmatter
   - Error scenarios and their fixes
   - API endpoints and response schemas

3. **Discard agent-specific framing:**
   - Remove "when asked to X, do Y" patterns
   - Remove cross-references to other skills (replace with doc links)
   - Remove API response schemas if they're duplicate of the API reference doc

### Step 3: Reformat for Human Audience

Transform the extracted content:

1. **Structure with clear headings** that answer user questions
   ("What fields does a knot file need?" not "Knot Definition File Format")

2. **Lead with examples** before explaining fields. Users learn from
   concrete examples first, then abstract rules.

3. **Use tables for reference material** (field lists, endpoint lists)
   but prose for explanations and workflows.

4. **Link domain terms** to `concepts.md` or the glossary on first use.

5. **Add verification steps** after each actionable section. Show the
   user how to confirm they did the right thing.

### Step 4: Write Target Document

Write the complete document following the restructured content:

1. Start with a brief **purpose statement** (one sentence: "This doc
   explains how to...")
2. Follow the restructured sections
3. End with **next steps** or cross-references to related docs
4. Keep line length under 80 characters for readability

### Step 5: Quality Check

Run the doc quality checklist (below) on the finished document. Fix
any issues before declaring it complete.

---

## Doc Quality Checklist

Run this checklist on every completed document. An item fails if the
condition is not met — fix before considering the doc done.

### Content Accuracy

- [ ] **All examples are runnable.** Every code block, file example,
      and curl command works as written. Test them or mark as
      "example only" if they depend on environment-specific values.
- [ ] **File paths are correct.** Paths reference the actual project
      layout (verify against the codebase, not assumptions).
- [ ] **API endpoints match the code.** Cross-reference with the router
      source. No stale or invented endpoints.
- [ ] **Field names and types are accurate.** Match the actual YAML
      frontmatter or API response schemas.

### Audience and Clarity

- [ ] **Each doc has a stated audience.** The first paragraph or heading
      makes clear who this doc is for.
- [ ] **Each doc has a clear goal.** A user can finish the doc knowing
      what they learned or accomplished.
- [ ] **No internal-only terminology without a glossary reference.**
      Every domain term (rig, loom, knot, strand, tie-off) either
      appears in context with a brief explanation or links to
      `concepts.md` / `project/domain-glossary.md`.
- [ ] **Agent-directed language is removed.** No "when asked to X, do
      Y" patterns. All instructions are user-directed.
- [ ] **Jargon is minimised** for end-user docs. Technical terms are
      explained on first use.

### Structure and Navigation

- [ ] **Links work.** All internal links (to other docs, to glossary,
      to API docs) resolve correctly. No broken relative paths.
- [ ] **Cross-references are helpful, not redundant.** Link to related
      docs where the user might go next, not where they just came from.
- [ ] **Headings form a logical hierarchy.** A user can skim headings
      and understand the document's structure.
- [ ] **Tables have clear column headers.** No ambiguous or empty headers.

### Completeness

- [ ] **Happy path is covered.** The main workflow is fully described
      with examples.
- [ ] **Common error cases are addressed.** Either in the doc itself or
      with a link to the troubleshooting guide.
- [ ] **Verification steps are included.** After each major action, the
      user knows how to confirm success.
- [ ] **Source traceability is maintained.** Every section can be traced
      back to a source artifact (skill, PRD, plan, code, or marked as
      original writing).

---

## Applying This Skill to a New Project

When adapting this methodology to a different project:

1. **Audit source artifacts.** List all internal documents (skills,
   glossaries, PRDs, plans, code) and map them to target docs.
2. **Define the target doc structure.** Adapt the directory layout to
   the project's domain. Keep the same categories: getting-started,
   concepts, configuration, workflows, API reference, troubleshooting,
   design guide, release notes.
3. **Identify the audience split.** Most tools have end-users and
   contributors. Some have additional audiences (operators,
   administrators).
4. **Extract and reformat** using the workflow above.
5. **Run the quality checklist** on each document.

---

## Cross-Reference

Related skills:

1. **project-planner skill** — create and maintain the plans that
   become release notes
2. **architecture-decision-record skill** — create ADRs that inform
   design guides
3. **design-pattern-record skill** — create DPRs that explain how
   things work (contributor docs)

This skill produces **user-facing documentation** from project
artifacts. The other skills produce the internal artifacts that serve
as source material.
