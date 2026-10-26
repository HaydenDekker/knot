# PRD: Shim Rig — Externalised Workflow Rig

## Problem

A rig (looms, knots, profiles) is the project's workflow definition. Today every
project that runs a workflow must keep its **own local copy** of the rig
directory. When a workflow is used by multiple projects and is relatively
stable, this duplicates maintenance: every workflow improvement has to be
re-applied to each project's copy, and the copies drift.

A local copy remains valuable for developing **bespoke workflows** (single
project, still changing). But once a workflow is shared and stable, the shared
definition should live in **one place** — the external rig — and each consuming
project should keep only what is genuinely project-specific: its rig
configuration (models, adapter, instruction links) and its runtime tree.

The glossary already positions the rig as "reusable source … versioned in its
own git repository", and `knot share <rig>` packages a rig for manual
transfer. What is missing is a **live reference**: a project-side rig that
points at an external rig and runs it, without a snapshot.

## Goals

- [ ] A project's rig can declare an external rig via one key in
  `rig/.workspace-agent-config.yaml` — `external-rig: <path>` (local path
  only: absolute, `~/...`, or project-root-relative)
- [ ] When the key is set (the rig is a **shim rig**), looms, knot files, and
  agent profiles are loaded **from the external rig**; the local rig's own
  looms/profiles are **ignored** with a startup warning listing every
  ignored item
- [ ] A missing, non-directory, or unreadable external path is a **hard
  startup error**: nothing starts, the service exits non-zero, and the
  diagnostic names the offending path and the config file that declared it
- [ ] The runtime tree stays project-local: `tie-offs/<rig-basename>/`
  under the consuming project (state, logs, queue unchanged in shape)
- [ ] Project-specific rig-level configuration (`models.yml`, the
  `agent-adapter` and instruction-file keys of
  `.workspace-agent-config.yaml`, `knot-template/` overrides) always comes
  from the **local** shim rig
- [ ] Hot-reload covers the external rig: loom/knot/profile edits in the
  external rig are picked up live by running projects, no restart
- [ ] Provenance is visible: the resolved external path is recorded in
  `state.json` and in the startup `[KNOT][CONFIG]` service-log lines
- [ ] A rig **without** the key behaves exactly as today (bespoke local
  workflow, unchanged)

## Non-Goals

- Git fetching/pinning by the service — a local path may *be* a git
  checkout, but pulling/updating it is a user or script concern, not a
  Knot concern (the `git:` ref form is a future extension)
- Per-loom or per-profile imports — a shim rig imports the external rig
  whole, or not at all
- Overriding/shadowing — a local knot/profile may not override an external
  one; the two sides are disjoint (local looms/profiles in a shim rig are
  ignored, not merged)
- Local knots joining an external loom
- Remote rig registries or URL-based rig sources
- Version management of external rigs (changelogs, compatibility checks) —
  provenance recording only

## User Stories

### Story 1: Run a shared workflow from one external rig

As a project owner, I keep a stable workflow rig in a shared location and
point my project's rig at it, so that the workflow is maintained in one place
instead of one copy per project.

**Scenarios:**

1. Given my project's `rig/.workspace-agent-config.yaml` sets
   `external-rig: ~/shared/factory-rig` and that directory contains looms,
   knots, and profiles, when I start Knot, then the project runs the
   external rig's workflow and the runtime tree lives at
   `tie-offs/rig/` under my project
2. Given my project is a shim rig and a strand triggers an external knot,
   when the knot completes, then the tie-off, commit, and state are recorded
   in the project's own runtime tree and git history
3. Given my project's `models.yml` defines `default: {provider: ..., model: ...}`
   and an external profile references `model-ref: default`, when the profile
   resolves, then the alias resolves against the **local** registry

### Story 2: Maintain the workflow once

As a workflow maintainer, I edit the external rig once, so that every
consuming project picks the change up without a per-project edit or restart.

**Scenarios:**

1. Given two running projects both shim `~/shared/factory-rig`, when I edit
   a knot file in the external rig, then both projects observe the change
   live (config reload), no restart
2. Given two projects shimming the same external rig, when I add a new loom
   to the external rig, then both projects discover it live

### Story 3: Fail clearly when the external rig is missing

As a project owner, I get an unambiguous error instead of a silently reduced
or broken rig when the external rig cannot be used.

**Scenarios:**

1. Given `external-rig: ~/shared/factory-rig` and the path does not exist,
   when I start Knot, then nothing starts: the service logs a clear
   diagnostic naming the path and the config file and exits non-zero
2. Given the local rig contains `planning-loom/` and `profiles/dev.md` while
   an external rig is set, when the service starts successfully, then the
   startup log warns and lists each local loom/profile that is ignored

### Story 4: Keep bespoke workflows local

As a project owner developing a workflow for one project, I keep it fully
local, so that a project with no external rig is unaffected by this feature.

**Scenarios:**

1. Given my project's rig has no `external-rig` key, when I start Knot, then
   behaviour is exactly as today — local looms, local profiles, local
   configuration
