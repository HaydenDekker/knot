# Plan 105: Explicit Per-Session Skill Assignment — the skill-supply seam

**Implementation Status:** ⬜ Planned (2026-11-13 drafted; 2026-11-13 extended
from the role-binding seam to explicit per-session skill assignment) —
drafted for owner review. Engine: config + frontmatter parsing, dispatch-time
resolution, CLI-flag delivery; tests + docs; released in a Knot version.
**No rig-content changes** — the served rig's prompts are another project's
files and are never touched here (engine/content split).

## Problem

Harness ISSUE-006 (superproject `project/issues/`): a project's
per-architecture behaviour requirements — the first named consumer is the
**adopted-theme declaration** chain (plan-author cites the theme contract,
phase items declare it, the coder builds to it) — must reach the right knots
**per project**. The rig may never reference a specific architecture, so
these behaviours cannot ride rig prompts by name.

Measured (2026-11-13, the rig today):

- **No skill seam at all.** No adapter passes `--skill` or `--no-skills`;
  every knot session runs with pi's **full discovery set** (the
  `~/.agents/skills` masters + the project `.agents/skills`) in the system
  prompt. Every knot gets every skill; nothing narrows, extends, or
  targets.
- **No project supply.** Project-specific behaviour skills have no
  declaration surface anywhere — rig content may not name an architecture,
  and the discovery set is user-machine-global, not per-project.

The engine gap this plan closes, in one sentence: **build a per-session,
explicit `--skill` list from three declaration surfaces and hand it to pi**,
keeping the engine path- and name-blind.

**pi primitives (measured 2026-11-13):** `--skill <path>` is repeatable and
**additive** (explicit paths still load under `--no-skills`); `--no-skills`
disables discovery. Explicit paths may be skill files or skill directories.
The engine's seam is therefore: compose the flag list, optionally add
`--no-skills` for narrowing. Skill loading into the system prompt is pi's
job; the seam composes *which* skills, not their placement (orthogonal to
`--append-system-prompt`, plan 103).

## Target

Three declaration surfaces, all **list-valued file references** (files or
directories; path rules per the existing instruction-link precedent —
absolute, relative-to-rig, `~/...`, `file://`; `http(s)://` refused):

1. **Rig config (rig-global, per-project).**
   `.workspace-agent-config.yaml` gains:

   ```yaml
   # explicit skills for every knot session in this rig
   skills:
     - path/to/the/projects/theme-contract-skill.md
   ```

   This is the ISSUE-006 seam: in a shim rig (plan 100) this file is
   **local and project-side** while looms/knots are served from the
   external workflow rig — so project-specific skills live here and the
   served workflow rig stays architecture-neutral. No role indirection is
   needed: the project-locality of this file *is* the boundary.
2. **Profile frontmatter.** `profiles/{name}.md` gains a `skills:` list —
   every knot served by that profile gets them.
3. **Knot frontmatter.** The knot definition file gains a `skills:` list —
   that specific knot only.

**Resolution at dispatch (per new session, fully dynamic):** the session's
skill list is the union of rig-global ∪ profile ∪ knot, **de-duplicated on
the resolved path**, delivered as repeated `--skill` arguments. The whole
chain is re-read at each new session — the `skills:` **lists themselves**
(the rig-config key follows the plan-102 per-session re-read precedent,
`FileSystemPromptInstructionSource`-style — not the startup-cached
`load_rig_config` path; knot/profile frontmatter follows the existing rig
file hot-reload) **and** the skill files (pi reads them at session start).
A project can therefore add, re-point, or remove a skill declaration and
the next session picks it up with **no service restart**; a session in
flight keeps the flags it was launched with (edits land on the next new
session, never mid-session — the plan-103 prompt-drift rule by analogy).

**Mode — additive vs exclusive (P0 decision):**

- *additive* (recommended default — current behaviour, pure extension):
  discovery set ∪ declared skills.
- *exclusive*: `--no-skills` + the declared set only — per-knot narrowing
  of today's full-set delivery.

Where the mode is set (rig-config key vs knot-frontmatter override) is a
P0 decision; the default must preserve today's behaviour for unmodified
rigs. **Empty declaration = all available:** with nothing declared at any
surface, the engine passes neither `--skill` nor `--no-skills` — pi's
normal discovery delivers the full set, unchanged (the zero-migration
property). Only exclusive mode ever *removes* skills, and it is opt-in.

**Failure is visible, never vacuous.** A declared skill path that cannot be
resolved/read: warn at dispatch (static-`[KNOT]`-style diagnostic,
`type=CONFIG` service-log line per the instruction-link precedent) and
proceed with the remainder — a declaration never silently evaluates to
nothing.

**Docs:** the config reference (the `skills` key, the frontmatter keys,
resolution + de-duplication semantics, the mode, diagnostics) + a short
skill-assignment guide (project perspective: declare once at rig level for
every knot, at profile level for a role, at knot level for one knot;
narrow with exclusive mode).

## Phases

- [ ] **P0 — design decisions (owner):** key naming (`skills` at all three
      surfaces, aligning with pi's own `skills` vocabulary); mode —
      additive default + where exclusivity is set (rig-config key and/or
      knot-level override); de-dup identity (resolved path) confirmation;
      shim-rig path rule (rig-config `skills` resolve against the **local**
      rig directory — the config side — knot/profile frontmatter `skills`
      against the **effective** rig directory — the served side); skill
      directories vs files (both accepted, matching pi).
- [ ] **P1 — engine:** config + frontmatter parsing, per-session resolution
      (union, de-dup, per-surface resolution roots), flag composition
      (`--skill` × n, `--no-skills` when exclusive), diagnostics; unit +
      integration tests.
- [ ] **P2 — e2e acceptance:** a probe rig declares skills at all three
      surfaces; the composed set appears in the delivered pi argv; an
      unresolvable path warns (and the rest still load); a re-pointed skill
      file is picked up fresh on the next session (no restart); exclusive
      mode delivers `--no-skills` and only the declared set.
- [ ] **P3 — docs + skills + release:**
      - *Repo docs:* `docs/configuration/rig-structure.md` (the `skills`
        config key), `docs/concepts.md` (explicit skill assignment),
        release notes carry the seam description (this is the artifact the
        rig-side follow-up measures against — the hand-off contract for
        the superproject issue); version bump.
      - *Knot skills* (plan-103 precedent: content summary per skill +
        frontmatter version bump, then deploy to `~/.agents/` with the
        AGENTS.md copy + diff-verify procedure):
        - **knot-create** — the `skills:` frontmatter key on knots and
          profiles (list of file/dir refs), and the rig-config `skills`
          key.
        - **knot-update** — a changelog entry for the releasing version:
          additive frontmatter/config schema change, no forced migration
          (unmodified files keep the full-discovery behaviour).
        - **knot-software-factory** — the "where technology knowledge
          lives" section: the seam by which a project supplies behaviour
          skills to factory knots (rig-config, project-local).
        - **knot-template** — one boundary sentence: the two instruction
          documents (placement of standing instructions) vs `skills:`
          (which skills load) are separate mechanisms; orthogonal.
        - **knot-inspect** — reading the diagnostics: the `type=CONFIG`
          missing-skill warning line.
      - *Explicitly not touched:* no skills-library content edits here —
        the served rig's skills are other projects' files (boundary note
        above).

## Boundary notes

- The engine and its docs **name no architecture, ever** — the engine
  resolves paths and composes flags; it interprets no skill content and
  carries no skill vocabulary. All vocabulary lives project-side.
- **Design superseded, seam kept:** the original draft's role-binding
  syntax (`@skill:<role>` in knot content + open role keys in
  `skill-imports`) is dropped. Roles would have re-introduced architecture
  naming into knot content; the explicit per-session list with the
  project-local rig-config surface achieves the same supply seam without it.
- No served-rig prompt edits, no skills-library edits: the skill *wording*
  in served rig content and the planner-skill norms are other projects'
  deliveries, bound to this one only through the issue record and the
  measured-artifact precondition each applies to itself.
