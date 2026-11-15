# Plan: Skill-First User Documentation

## Problem

`docs/` was built (plan [`user-documentation.md`](user-documentation.md), 2026-06-18)
by extracting skill content and reformatting it for human readers. That
extraction-by-copy has two failure modes, both now visible:

- **Drift** — `docs/getting-started.md`'s skill-deploy loop lists 10
  sub-skills; the canonical loop in `AGENTS.md` lists 14 (`knot-lint`,
  `knot-visualise`, `knot-template`, `knot-software-factory` missing).
  `getting-started.md` still carries the old manual `knot`-with-stderr
  startup instructions that `knot-start` superseded. `configuration/`
  and `design-guide.md` describe formats that only partially match the
  current skills.
- **Wrong reader model** — the docs speak to a human who reads
  procedure and types commands. The actual user interface is the
  agent: a user *asks their agent* to do things, and the skills are
  what the agent does. The docs duplicate what the skills already say
  instead of telling the user what to ask.

## Target

Skill-first documentation. **The user interface is the agent.**

- Every doc step is a *journey step*, not a procedure: **ask your
  agent to …** + one skill + what the user watches + what the agent
  reports.
- Docs contain **zero skill procedure**: no frontmatter field tables,
  no error tables, no deploy bash loops. If a doc step needs a
  procedure, the procedure goes in the skill.
- The repo is **self-documenting at agent level**: `AGENTS.md` is the
  single canonical source for build, install, and skill
  deployment/verification. Users never need a "how to install" doc —
  they point their agent at the repo.
- Narrative-only docs (concepts, release notes) stay; overlapping
  reference docs are retired to a pointer page.

## Implementation Status: ⬜ Not started

## Guiding principles

1. **Docs are the journey; skills are the procedure.** A doc names a
   skill and says what to watch; it never restates what the skill
   does mechanically.
2. **`AGENTS.md` owns install/deploy.** The 14-sub-skill + master +
   `knot-template` companion-file deploy loop and its diff
   verification live only in `AGENTS.md`. Any copy elsewhere is a
   drift bug.
3. **One skill per doc step.** A step that needs two skills is two
   steps.

## Phases

### Phase 0: Gap audit (done in planning discussion, 2026-11-02)

| Getting-started step | Backing skill | Audit result |
|---|---|---|
| 1. Clone & set up from the repo | *(none — by design)* | ✅ Narrative only: agent clones, reads the repo's `AGENTS.md`, installs. No install skill needed. Kill the duplicated deploy loop in `docs/getting-started.md`. |
| 2. Initialise the project | `knot-init` | ✅ Skill exists; doc delegates. |
| 3. Create a workflow | `knot-create` | ✅ Skill exists; doc delegates. |
| 4. Start Knot | `knot-start` | ✅ Skill exists; doc still carries superseded manual-start instructions — delete. |
| 5. Trigger work | `knot-dispatch` | ✅ Skill exists; not currently a doc step — add. |
| 6. View the rig | `knot-visualise` | ✅ Skill exists; not currently a doc step — add. |

Conclusion: the only doc steps missing are 5 and 6; the only missing
*capability* is none — everything is a doc-side rewrite. No new skill
required.

### Phase 1: Rewrite `docs/getting-started.md` as the six-step journey

- Structure: one section per step. Each section is at most:
  1. the ask (one or two sentences, phrased so a user can paste it
     verbatim to their agent),
  2. the skill the agent will use (named, linked),
  3. what the user watches (files appearing, `tie-offs/<rig>/state.json`,
     service-log lines, the HTML graph) and what the agent reports back.
- Step 1 (setup): "Point your agent at the Knot repository and ask it
  to build and install Knot and install the Knot skills. It will read
  the repository's `AGENTS.md`, which carries the full
  install/deploy procedure." No bash.
- Step 3 keeps "Hello Knot" (or a slimmer successor) as the example
  workflow created via `knot-create`.
- Steps 5–6 are new: dispatch a strand and watch it process
  (`knot-dispatch`); render the topology and open the HTML file
  (`knot-visualise`).
- Remove: the 10-skill deploy loop + verification loop, the manual
  `knot`-with-stderr startup block, the skills-reference table at the
  end (or replace with one-line pointers, since the master router is
  the real index).
- State the floor: Knot version and "skills are versioned per their
  frontmatter; ask your agent to check compatibility" (via
  `knot-update`).

### Phase 2: Retire the overlapping reference docs

| Doc | Action |
|---|---|
| `docs/configuration/rig-structure.md`, `knots.md`, `profiles.md` | Collapse into one `docs/configuration.md`: a page of links — file formats and CRUD live in `knot-create`; rig anatomy lives in `concepts.md`. Keep only genuinely narrative content (e.g. how discovery works, in a paragraph). |
| `docs/design-guide.md` | Retire to a pointer: "designing looms and knots is the `knot-design` skill." Keep a one-paragraph summary if it adds anything the skill doesn't. |
| `docs/workflows/review-workflow.md`, `file-generation-workflow.md` | Rewrite as "ask your agent to…" journeys in the Phase-1 voice (agent does the work; user watches). |
| `docs/troubleshooting.md` | Keep, but delete anything that restates a skill's error-handling table; keep only what skills don't cover (observing the rig, reading the service log — `knot-inspect` / `knot-manage` pointers). |
| `docs/concepts.md` | Keep (narrative; no skill source). |
| `docs/release-notes.md` | Keep (plan-derived; feeds `knot-update`). |

Acceptance: after Phase 2, no file under `docs/` contains a frontmatter
field table, an error-handling table, or a multi-line bash block that
reproduces a skill procedure.

### Phase 3: Drift guard (advisory)

The original plan shipped a `project-user-documentation` skill
(extract skills → docs per release). That methodology is superseded by
this plan's model; update or retire it so future agents don't
re-extract.

Concretely:

- Extend `knot-lint` (skill `knot-lint`, script `scripts/knot-lint.py`)
  with a check family **K6 (docs-drift)** that flags, in `docs/`:
  - bash loops iterating over `knot-*` skill names (deploy-loop
    copies),
  - frontmatter field tables for knots/profiles,
  - skill lists (prose or tables) that don't match the skill set
    present in `.agents/skills/`.
- Add the "no skill procedure in docs" rule to
  `knot-design` (or the docs guidance) so future doc edits follow the
  boundary.
- `AGENTS.md`: add a one-line note in its docs/skills area that
  `docs/` is the agent-facing journey and skills are the procedure
  source of truth (keep the deploy loop in `AGENTS.md` as the single
  canonical copy — that part is unchanged and is exactly what makes
  Phase 0's "no install skill" decision hold).

## Out of scope

- No new skills in this plan (Phase 0 found no capability gaps).
- No skill rewrites — this plan touches `docs/`, `AGENTS.md` (additive
  note), the lint script, and the retired `project-user-documentation`
  skill only.
- No rig execution in this repository (per `AGENTS.md`: verify with
  `cargo test` / `cargo clippy` and the mock-CLI harness; no live rig
  runs here).

## Verification

- Phase 1–2: a fresh-reader walk — paste each "ask" sentence into an
  agent session against a clean checkout; each must resolve to exactly
  one skill and be completable without consulting anything else in
  `docs/`.
- Phase 3: `python3 scripts/knot-lint.py --selftest` passes with the
  K6 fixtures (one good docs tree, one docs tree containing a
  deploy-loop copy); `cargo test` / `cargo clippy` clean for the
  script's Rust-side changes if any.

## Backwards compatibility

Old doc paths are retired, not renamed in place; README links and any
skill references to `docs/*` paths are updated in the same commit as
each retirement (grep for `docs/` across `.agents/skills/` and the
README before deleting).
