# ADR-013: Shim Rig — Externalised Workflow Rig with Local Project Shim

**Date**: 2026-10-27
**Status**: Accepted

## Context

A rig (looms, knot files, agent profiles) is the project's workflow
definition. Before Knot 0.64.0 every project that ran a workflow kept
its own **local copy** of the rig directory. When one workflow is used
by multiple projects and is relatively stable, this duplicates
maintenance: every workflow improvement must be re-applied to each
project's copy, and the copies drift. `knot share <rig>` (zip
packaging) ships a rig, but a snapshot is still a copy.

A local copy remains valuable for **bespoke workflows** — single
project, still changing. The missing capability was a **live reference**
to a shared external rig, while keeping what is genuinely
project-specific local: rig-level configuration (`models.yml`,
adapter, instruction links) and the runtime tree.

The glossary already positioned the rig as "reusable source … versioned
in its own git repository" — the shim makes that statement operational.

## Decision

A project-local rig may declare an **external rig** via one key in
`rig/.workspace-agent-config.yaml`:

```yaml
external-rig: shared/factory-rig   # absolute, ~/..., or project-root-relative
```

A rig with the key is a **shim rig**. Six decisions define the contract:

1. **Local path only.** The external rig is a local directory
   (absolute, `~/...`, or relative to the consuming project root).
   The service never fetches; a git checkout is a valid target, but
   pulling/updating it is a user or script concern. The `git:` ref
   form (with resolved-SHA pinning) is a deliberate future extension,
   not a present one.

2. **Whole-rig import.** A shim rig imports the external rig whole.
   No per-loom or per-profile selection — the merge bookkeeping of
   partial imports was rejected as cost without proportional benefit.

3. **No overriding; disjoint sides.** When the key is set, the
   workflow comes from the external rig *or* the local rig, never
   merged. Local `*-loom/` directories and `profiles/*.md` in a shim
   rig are **ignored**, each listed in a `[KNOT][CONFIG]` startup
   warning. A local knot cannot join an external loom. (Shadowing —
   local-over-external override — was considered and rejected for v1:
   it invites silent drift between the shared workflow and per-project
   forks of individual knots.)

4. **Hard fail on an unresolvable path.** A missing, non-directory,
   unreadable, or empty `external-rig` value is a startup error:
   nothing starts, the `[KNOT][CONFIG] ERROR` diagnostic names the
   path and the declaring config file, exit is non-zero. A silent
   fallback to the local stub would run a project on the wrong
   workflow unnoticed.

5. **Identity stays local; the external path is provenance.** The
   runtime tree remains `tie-offs/<local-rig-basename>/` under the
   consuming project; `state.json`'s `rig_path` is the local rig
   directory and the resolved external path is recorded in the
   additive `external_rig` field (skipped for plain rigs). Two
   projects shimming the same external rig need zero coordination —
   their runtime trees are disjoint by construction. Relative
   `strand-dir` paths resolve against the **consuming project root**
   (via an explicit resolution base on the loom repository), so
   strands belong to the project, not the external rig's
   neighbourhood.

6. **Project-side config always local.** `models.yml`, the
   `agent-adapter` / instruction-file keys, and `knot-template/`
   overrides come from the local shim rig. The external rig's own
   `models.yml` / `.workspace-agent-config.yaml` are never loaded.
   External profiles' `model-ref` aliases resolve against the
   consuming project's registry — the alias indirection (plan 069)
   was designed for exactly this seam.

Hot-reload follows the effective (external) rig directory: edits, new
looms, and knot changes in the external rig are picked up live by
every consuming project, with the same debounce/reload semantics as
local-rig edits. `knot share` on a shim packs the external rig's
contents under the local rig's name (with a CLI note); a broken shim
reference fails the share.

## Consequences

**Positive:**

- One workflow edit serves N projects live — the maintenance
  duplication is gone without giving up the local-bespoke path (a rig
  without the key behaves byte-for-byte as before).
- A natural promotion lifecycle emerges: *local bespoke → shared
  external → shim in N projects*.
- The trust model is unchanged from pulled skills: an external rig is
  prompt text the user chose to point at; provenance in `state.json`
  is the audit hook.

**Negative / accepted:**

- The external rig's own config files are inert — a user who expects
  them to travel with the workflow must learn the seam (documented in
  the glossary term **Shim Rig** and the skill notes).
- No pinning: a floating external checkout can change behaviour
  between runs. Accepted for the local-first scope; the `git:` ref
  form addresses it later.
- The warn-and-ignore rule means a shim rig carrying local workflow
  files is always a visible smell, never a silent merge — deliberate,
  but it makes half-migrated rigs noisy until cleaned up.

## References

- Plan: [100-shim-rig](../plans/100-shim-rig/shim-rig-plan.md)
- PRD: [prd-shim-rig](../prds/prd-shim-rig.md)
- Glossary: **Shim Rig** (`knot-init/knot-glossary.md`)
- Changelog: `knot-update` skill, Knot 0.64.0
