# Plan 105: Project Skill Imports and Role-Bound Knot Bindings — the skill-supply seam

**Implementation Status:** ⬜ Planned (2026-11-13) — drafted for owner
review. Engine: config semantics + dispatch-time resolution + injection;
tests + docs; released in a Knot version. **No rig-content changes** —
the served rig's prompts are another project's files and are never
touched here (engine/content split).

## Problem

Harness ISSUE-006 (superproject `project/issues/`): a project's
per-architecture behaviour requirements — the first named consumer is
the **adopted-theme declaration** chain (plan-author cites the theme
contract, phase items declare it, the coder builds to it) — must reach
the right knots **per project**. The rig may never reference a specific
architecture, so these behaviours cannot ride rig prompts by name.

Measured (2026-11-13, the rig today): skills reach knots **only** as
hard-coded names in rig prompt content, and there is **no seam** by
which a project supplies its own behaviour skills or maps them onto
knots — zero hits for any project-declared skill-import shape. The
injection machinery itself exists (the two fixed-scope instruction
documents, plans 102/103 — per-section freshness, fallback to embedded
default), which is why this is an **extension of existing engine
behaviour, not new machinery**.

**The engine cannot today execute what the content needs to express**:
no config semantics for skill imports, no role→skill resolution at
dispatch. That is the engine gap this plan closes; the rig-side wording
follows in the rig's own project, gated on **this release's artifact**
(the config docs), never on this plan's number.

## Target

1. **Config declaration (imports).** The project's rig-level agent
   config gains a declared set of **behaviour-skill imports keyed by
   role**, e.g.:

   ```yaml
   # behaviour-skill imports — role: skill-file
   skill-imports:
     theme-contract: path/to/the/projects/theme-contract-skill.md
   ```

   Role keys are **open strings** (the engine knows no vocabulary; it
   resolves, it does not interpret) — the engine stays
   architecture-blind. Supply-neutral: reference-born and
   project-invented skills are imported identically.
2. **Dispatch-time resolution (bindings).** Knot/strand content may
   bind a role (`@skill:theme-contract` form decided at P0); at
   dispatch the engine resolves role → imported skill for **that
   project** and injects the skill file into the knot's context —
   freshness and injection scope per the plan-102 precedent (pulled
   fresh per new session).
3. **Missing binding is visible, never vacuous.** A knot binds a role
   the project declared no skill for: the engine **warns at dispatch**
   (static-`[KNOT]`-style diagnostic) and proceeds per the P0-ruled
   policy (default: proceed with an explicit "no binding" marker in
   context — a declaration must never silently evaluate to nothing).
4. **Docs**: the config reference (keys, resolution semantics,
   diagnostics) + a short skill-supply guide (project perspective:
   declare once, every bound knot picks it up).

## Phases

- [ ] **P0 — design decisions (owner):** the binding syntax; injection
      scope (every knot that binds vs strand-kind scopes); missing-
      binding policy confirmation; role-key vocabulary openness;
      interaction with `--append-system-prompt` (plan 103) — injected
      skill vs appended instruction precedence.
- [ ] **P1 — engine:** config parsing + resolution + injection +
      diagnostics, unit + integration tests.
- [ ] **P2 — e2e acceptance:** a probe rig declares an import; a bound
      knot sees it; an unbound role warns; a changed skill file is
      picked up fresh (no restart).
- [ ] **P3 — docs + release:** config reference + guide; version bump,
      release notes carry the seam description (this is the artifact
      the rig plan's P0 pane measures).

## Boundary notes

- The engine and its docs **name no architecture, ever** — roles are
  opaque strings; the vocabulary lives project-side.
- No served-rig prompt edits, no skills-library edits: the role-bound
  *wording* in rig content and the planner-skill norms are other
  projects' deliveries, bound to this one only through the issue record
  and the measured-artifact precondition each applies to itself.
